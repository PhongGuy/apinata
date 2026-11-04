// `bru.cookies` over the host's cookie jar. Reads are synchronous and ask the host every time,
// so a cookie stored by a script's own send shows up. Writes and the `jar()` handle return
// Promises; a trailing function argument is called as `cb(null, result)` or `cb(err, null)`.
(() => {
  const read = globalThis.__cookies;
  const host = globalThis.__cookieOp;
  delete globalThis.__cookies;
  delete globalThis.__cookieOp;
  const list = () => JSON.parse(read());
  const last = (name) => list().filter((c) => c.key === name).pop();

  // One host call. A refused cookie still succeeds, with one warning that never holds its value.
  const op = (request) => {
    const answer = JSON.parse(host(JSON.stringify(request)));
    if (answer.w) console.warn(answer.w);
    if ("e" in answer) throw new Error(answer.e);
    return answer.v;
  };

  // Wraps a synchronous body as an async method; with a callback, an error goes to the
  // callback and the Promise still resolves.
  const later = (body) => async (...args) => {
    const cb = typeof args[args.length - 1] === "function" ? args.pop() : undefined;
    if (!cb) return body(...args);
    let result;
    try {
      result = body(...args);
    } catch (err) {
      await cb(err, null);
      return undefined;
    }
    await cb(null, result);
    return result;
  };

  const expiry = (at) => {
    if (at === undefined || at === null || at === Infinity || at === "Infinity") return null;
    const ms = (at instanceof Date ? at : new Date(at)).getTime();
    return Number.isFinite(ms) ? ms : null;
  };
  const text = (v) => (v === undefined || v === null || v === "" ? null : String(v));

  // `name` stands in for a missing `key`; `url` null means the request URL.
  const setOne = (url, obj) => {
    const key = obj && typeof obj === "object" ? obj.key || obj.name : undefined;
    if (!key) throw new Error("cookieObject.key (name) is required");
    const maxAge = obj.maxAge === undefined || obj.maxAge === null ? NaN : Number(obj.maxAge);
    const cookie = {
      key: String(key),
      value: obj.value === undefined || obj.value === null ? "" : String(obj.value),
      domain: text(obj.domain),
      path: text(obj.path),
      expires: expiry(obj.expires),
      maxAge: Number.isFinite(maxAge) ? maxAge : null,
      secure: !!obj.secure,
      httpOnly: !!obj.httpOnly,
      hostOnly: obj.hostOnly === true,
    };
    op({ op: "set", url, cookie });
  };
  const needUrl = (url) => {
    if (!url) throw new Error("URL is required");
  };
  const needUrlAndName = (url, name) => {
    if (!url || !name) throw new Error("URL and cookie name are required");
  };
  const listFor = (url) => op({ op: "list", url: String(url) });
  // Of the cookies sent to `url` with this name, the one with the longest path.
  const pick = (url, name) =>
    listFor(url)
      .filter((c) => c.key === name)
      .reduce((best, c) => (!best || c.path.length > best.path.length ? c : best), null);

  const jar = () => ({
    getCookie: later((url, name) => {
      needUrlAndName(url, name);
      return pick(url, name);
    }),
    getCookies: later((url) => {
      needUrl(url);
      return listFor(url);
    }),
    hasCookie: later((url, name) => {
      needUrlAndName(url, name);
      return pick(url, name) !== null;
    }),
    setCookie: later((url, nameOrCookie, value) => {
      needUrl(url);
      if (typeof nameOrCookie === "string") {
        if (!nameOrCookie) throw new Error("Cookie name is required");
        setOne(String(url), { key: nameOrCookie, value: typeof value === "string" ? value : "" });
      } else if (nameOrCookie && typeof nameOrCookie === "object") {
        setOne(String(url), nameOrCookie);
      } else {
        throw new Error("Invalid arguments passed to setCookie");
      }
    }),
    setCookies: later((url, cookies) => {
      needUrl(url);
      if (!Array.isArray(cookies)) throw new Error("setCookies expects an array of cookie objects");
      for (const cookie of cookies) setOne(String(url), cookie);
    }),
    deleteCookie: later((url, name) => {
      needUrlAndName(url, name);
      op({ op: "delete", url: String(url), name: String(name) });
    }),
    deleteCookies: later((url) => {
      needUrl(url);
      op({ op: "deleteAll", url: String(url) });
    }),
    clear: later(() => {
      op({ op: "clear" });
    }),
  });

  const add = later((cookie) => {
    if (!cookie || typeof cookie !== "object") throw new Error("cookieObj must be a non-null object");
    setOne(null, cookie);
  });
  const remove = later((name) => {
    if (name) op({ op: "delete", url: null, name: String(name) });
  });
  bru.cookies = {
    get: (name) => last(name)?.value,
    has: (name, value) => list().some((c) => c.key === name && (value === undefined || c.value === value)),
    count: () => list().length,
    one: (name) => last(name),
    all: () => list(),
    toObject: () => Object.fromEntries(list().map((c) => [c.key, c.value])),
    add,
    upsert: add,
    remove,
    delete: remove,
    clear: later(() => {
      op({ op: "deleteAll", url: null });
    }),
    jar,
  };
})();
