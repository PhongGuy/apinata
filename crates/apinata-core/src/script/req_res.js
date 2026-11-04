// Builds `req`, and `res` when the host offers it, over the host's JSON dispatch functions,
// which it takes off the global object. Argument coercion follows the reference's shims.
(() => {
  const unsupported = __apinata.unsupported;
  const bridge = (host) => (op, ...args) => {
    const answer = JSON.parse(host(op, JSON.stringify(args)));
    if ("e" in answer) throw new Error(answer.e);
    if ("u" in answer) unsupported(answer.u);
    return answer.v;
  };

  const reqHost = bridge(globalThis.__req);
  delete globalThis.__req;
  const str = (v) => String(v);
  // Objects go over the wire as JSON text whatever the content type, primitives as their text.
  // Like the reference, an absent body (undefined or null) is sent as no body at all.
  const bodyText = (data) => (data == null ? "" : typeof data === "object" ? JSON.stringify(data) : str(data));
  const getBody = (options = {}) => {
    const { text, json } = reqHost("getBody");
    if (options.raw || !json) return text;
    try { return JSON.parse(text); } catch (e) { return text; }
  };
  const snapshotBody = () => {
    try { return getBody(); } catch (e) { return undefined; }
  };

  // The snapshot fields are read once, at phase start; the getters read the request as edited.
  const req = {
    url: reqHost("getUrl"),
    method: reqHost("getMethod"),
    headers: reqHost("getHeaders"),
    body: snapshotBody(),
    timeout: reqHost("getTimeout"),
    name: reqHost("getName"),
    pathParams: reqHost("getPathParams"),
    tags: reqHost("getTags"),

    getUrl: () => reqHost("getUrl"),
    setUrl: (url) => reqHost("setUrl", str(url)),
    getHost: () => reqHost("parts").host,
    getPath: () => reqHost("parts").path,
    getQueryString: () => reqHost("parts").query,
    getMethod: () => reqHost("getMethod"),
    setMethod: (method) => reqHost("setMethod", str(method)),
    getName: () => reqHost("getName"),
    getAuthMode: () => reqHost("getAuthMode"),
    getPathParams: () => reqHost("getPathParams"),
    getTags: () => reqHost("getTags"),
    getExecutionMode: () => reqHost("getExecutionMode"),
    getHeaders: () => reqHost("getHeaders"),
    getHeader: (name) => reqHost("getHeader", str(name)),
    setHeader: (name, value) => reqHost("setHeader", str(name), str(value)),
    deleteHeader: (name) => reqHost("deleteHeader", str(name)),
    deleteHeaders: (names) => names.forEach((name) => reqHost("deleteHeader", str(name))),
    setHeaders: (headers) => {
      const out = {};
      for (const [name, value] of Object.entries(headers || {})) out[name] = str(value);
      reqHost("setHeaders", out);
    },
    getBody,
    setBody: (data, options = {}) => reqHost("setBody", options.raw && typeof data === "string" ? data : bodyText(data)),
    getTimeout: () => reqHost("getTimeout"),
    setTimeout: (ms) => reqHost("setTimeout", Number(ms) || 0),
    setMaxRedirects: (max) => reqHost("setMaxRedirects", Number(max) || 0),
    disableParsingResponseJson: () => unsupported("req.disableParsingResponseJson"),
    onFail: () => unsupported("req.onFail"),
  };
  globalThis.req = req;

  if (!globalThis.__res) return;
  const resHost = bridge(globalThis.__res);
  delete globalThis.__res;
  const query = globalThis.__query;
  delete globalThis.__query;
  let data = resHost("data");
  const body = () => {
    if (!("body" in data)) __apinata.unsupportedBecause("the response body was truncated");
    return data.body;
  };

  // `res` is a function so `typeof res` matches the reference: `res(path, ...filters)` queries the body.
  const res = function res(path, ...filters) { return query(body(), path, ...filters); };
  const field = (name, get) => Object.defineProperty(res, name, { get, enumerable: true });
  field("status", () => data.status);
  field("statusText", () => data.statusText);
  field("headers", () => data.headers);
  field("body", body);
  field("responseTime", () => data.responseTime);
  field("url", () => data.url);
  Object.assign(res, {
    getStatus: () => data.status,
    getStatusText: () => data.statusText,
    getHeader: (name) => (typeof name === "string" ? data.headers[name.toLowerCase()] : null),
    getHeaders: () => data.headers,
    getBody: body,
    getResponseTime: () => data.responseTime,
    getUrl: () => data.url,
    getSize: () => ({ ...data.size }),
    // The value crosses as JSON, which also gives the deep copy the reference makes.
    setBody: (value) => {
      resHost("setBody", value === undefined ? null : value);
      data = resHost("data");
    },
  });
  globalThis.res = res;
})();
