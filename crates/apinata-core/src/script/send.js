// Builds `bru.sendRequest`, `axios` and `require("axios")` over the async host send, which it
// takes off the global object. The shared `send` stays on `__apinata` for later libraries.
(() => {
  const host = globalThis.__send;
  delete globalThis.__send;

  // axios's config shape: a string is the URL, and object data goes as JSON unless the caller
  // chose a content type; string data goes as is.
  const normalise = (config) => {
    if (typeof config === "string") config = { url: config };
    config = config || {};
    const headers = [];
    for (const [name, value] of Object.entries(config.headers || {})) {
      if (value !== undefined && value !== null) headers.push([name, String(value)]);
    }
    const out = {
      method: String(config.method || "get").toUpperCase(),
      url: String(config.url ?? ""),
      headers,
    };
    const data = config.data;
    if (typeof data === "string") out.body = data;
    else if (data !== undefined && data !== null) {
      out.body = JSON.stringify(data);
      if (!headers.some(([name]) => name.toLowerCase() === "content-type")) {
        headers.push(["Content-Type", "application/json"]);
      }
    }
    if (config.timeout) out.timeout = Number(config.timeout);
    return out;
  };

  // A non-2xx status rejects as axios's `validateStatus` does, carrying the response.
  const send = async (config) => {
    const answer = JSON.parse(await host(JSON.stringify(normalise(config))));
    if ("e" in answer) throw new Error(answer.e);
    const res = answer.v;
    if (res.status < 200 || res.status >= 300) {
      const e = new Error("Request failed with status code " + res.status);
      e.status = res.status;
      e.code = res.status < 500 ? "ERR_BAD_REQUEST" : "ERR_BAD_RESPONSE";
      e.response = res;
      throw e;
    }
    return res;
  };

  // With a callback, the callback sees the outcome first: a success resolves the response, a
  // failure resolves `undefined`, and a callback that throws rejects with its own error.
  bru.sendRequest = async (config, cb) => {
    if (typeof cb !== "function") return send(config);
    let res;
    try {
      res = await send(config);
    } catch (err) {
      await cb(err, null);
      return undefined;
    }
    await cb(null, res);
    return res;
  };

  const axios = (config) => send(config);
  for (const method of ["get", "delete"]) {
    axios[method] = (url, config) => send({ ...config, method, url });
  }
  for (const method of ["post", "put", "patch"]) {
    axios[method] = (url, data, config) => send({ ...config, method, url, data });
  }
  globalThis.axios = axios;

  // Each library answers its own module names and hands the rest to the previous `require`.
  const previous = globalThis.require;
  globalThis.require = (name) => (String(name) === "axios" ? axios : previous(name));

  __apinata.send = send;
})();
