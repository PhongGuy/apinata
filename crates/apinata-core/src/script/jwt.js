// `require("jsonwebtoken")` and a lazy `jwt` global over the `__jwt` host op. Options, claims,
// timespans and error wording follow jsonwebtoken 9.0.3 and ms 2.1.3 (MIT). Keys are read through
// `crypto`, as the library's are; the host owns base64url and the signatures.
(() => {
  const host = globalThis.__jwt;
  delete globalThis.__jwt;
  const { KeyObject, createPrivateKey, createPublicKey, createSecretKey } = globalThis.crypto;
  const call = (op, ...args) => {
    const answer = JSON.parse(host(op, JSON.stringify(args)));
    if ("e" in answer) throw new Error(answer.e);
    return answer.v;
  };

  class JsonWebTokenError extends Error {
    constructor(message, error) {
      super(message);
      this.name = "JsonWebTokenError";
      if (error) this.inner = error;
    }
  }
  class TokenExpiredError extends JsonWebTokenError {
    constructor(message, expiredAt) {
      super(message);
      this.name = "TokenExpiredError";
      this.expiredAt = expiredAt;
    }
  }
  class NotBeforeError extends JsonWebTokenError {
    constructor(message, date) {
      super(message);
      this.name = "NotBeforeError";
      this.date = date;
    }
  }

  // ms 2.1.3 `parse`: milliseconds for a timespan string, undefined when it does not match.
  const UNITS = {};
  for (const [size, names] of [
    [31557600000, "years year yrs yr y"], [604800000, "weeks week w"], [86400000, "days day d"],
    [3600000, "hours hour hrs hr h"], [60000, "minutes minute mins min m"],
    [1000, "seconds second secs sec s"], [1, "milliseconds millisecond msecs msec ms"],
  ]) for (const name of names.split(" ")) UNITS[name] = size;
  const ms = (str) => {
    if (str.length > 100) return undefined;
    const match = /^(-?(?:\d+)?\.?\d+) *(milliseconds?|msecs?|ms|seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h|days?|d|weeks?|w|years?|yrs?|y)?$/i.exec(str);
    if (!match) return undefined;
    return parseFloat(match[1]) * UNITS[(match[2] || "ms").toLowerCase()];
  };
  const timespan = (time, iat) => {
    const timestamp = iat || Math.floor(Date.now() / 1000);
    if (typeof time === "string") {
      const milliseconds = ms(time);
      return milliseconds === undefined ? undefined : Math.floor(timestamp + milliseconds / 1000);
    }
    return typeof time === "number" ? timestamp + time : undefined;
  };

  const isString = (v) => typeof v === "string";
  const isInteger = (v) => Number.isInteger(v);
  const isPlainObject = (v) => {
    if (v === null || typeof v !== "object" || Object.prototype.toString.call(v) !== "[object Object]") return false;
    const proto = Object.getPrototypeOf(v);
    return proto === null || (Object.prototype.hasOwnProperty.call(proto, "constructor") && proto.constructor === Object);
  };
  const SUPPORTED_ALGS = ["RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384", "ES512", "HS256", "HS384", "HS512", "none"];
  const timespanMessage = (name) => `"${name}" should be a number of seconds or string representing a timespan`;
  const SIGN_OPTIONS = {
    expiresIn: [(v) => isInteger(v) || (isString(v) && v), timespanMessage("expiresIn")],
    notBefore: [(v) => isInteger(v) || (isString(v) && v), timespanMessage("notBefore")],
    audience: [(v) => isString(v) || Array.isArray(v), '"audience" must be a string or array'],
    algorithm: [(v) => SUPPORTED_ALGS.includes(v), '"algorithm" must be a valid string enum value'],
    header: [isPlainObject, '"header" must be an object'],
    encoding: [isString, '"encoding" must be a string'],
    issuer: [isString, '"issuer" must be a string'],
    subject: [isString, '"subject" must be a string'],
    jwtid: [isString, '"jwtid" must be a string'],
    noTimestamp: [(v) => typeof v === "boolean", '"noTimestamp" must be a boolean'],
    keyid: [isString, '"keyid" must be a string'],
    mutatePayload: [(v) => typeof v === "boolean", '"mutatePayload" must be a boolean'],
    allowInsecureKeySizes: [(v) => typeof v === "boolean", '"allowInsecureKeySizes" must be a boolean'],
    allowInvalidAsymmetricKeyTypes: [(v) => typeof v === "boolean", '"allowInvalidAsymmetricKeyTypes" must be a boolean'],
  };
  const CLAIMS = {
    iat: [(v) => typeof v === "number", '"iat" should be a number of seconds'],
    exp: [(v) => typeof v === "number", '"exp" should be a number of seconds'],
    nbf: [(v) => typeof v === "number", '"nbf" should be a number of seconds'],
  };
  const validate = (schema, allowUnknown, object, parameterName) => {
    if (!isPlainObject(object)) throw new Error(`Expected "${parameterName}" to be a plain object.`);
    for (const key of Object.keys(object)) {
      const validator = schema[key];
      if (!validator) {
        if (!allowUnknown) throw new Error(`"${key}" is not allowed in "${parameterName}"`);
        continue;
      }
      if (!validator[0](object[key])) throw new Error(validator[1]);
    }
  };

  // A key goes to the host as a string or as the bytes of a typed array.
  const isBytes = (v) => v instanceof ArrayBuffer || ArrayBuffer.isView(v);
  const material = (key) => {
    if (typeof key === "string") return key;
    if (key instanceof ArrayBuffer) return Array.from(new Uint8Array(key));
    if (ArrayBuffer.isView(key)) return Array.from(new Uint8Array(key.buffer, key.byteOffset, key.byteLength));
    return undefined;
  };

  // What the rest of a call needs of an opened key: its types, its PEM, and its curve or size.
  const typed = (key) => {
    if (key.type === "secret") return { type: "secret", bytes: material(key.export()) };
    const bytes = key.export({ type: key.type === "private" ? "pkcs8" : "spki", format: "pem" });
    const pss = key.asymmetricKeyType === "rsa-pss" ? key.asymmetricKeyDetails : undefined;
    return { type: key.type, kind: key.asymmetricKeyType, bytes, pss, ...call("details", bytes) };
  };
  const secretOf = (key) => {
    if (typeof key === "string") return { type: "secret", bytes: key };
    try {
      return typed(createSecretKey(key));
    } catch (_) {
      return undefined;
    }
  };
  // Refusals that name a key Node would open. Without a passphrase Node opens no encrypted key.
  const PEM_MARKER = "-----BEGIN";
  const NOT_A_KEY = "Unsupported key: text with a PEM marker is never a secret, and this is no key that opens here (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519)";
  const NAMED = /^Unsupported key: (?!expected a PEM|the encrypted private key could not be read)/;
  const NAMED_PLAIN = /^Unsupported key: (?!expected a PEM|.*encrypted)/;
  // As jsonwebtoken does, what opens is typed and other strings or bytes are secrets, with one
  // exception that closes key confusion: text with a PEM marker is a key or a refusal, never HMAC.
  const readKey = (key, side) => {
    if (key instanceof KeyObject) return typed(key);
    if (typeof key === "string" && !key.includes(PEM_MARKER)) return secretOf(key);
    const wrapped = key !== null && typeof key === "object" && !isBytes(key);
    try {
      return typed((side === "private" ? createPrivateKey : createPublicKey)(key));
    } catch (error) {
      if ((wrapped && key.passphrase !== undefined ? NAMED : NAMED_PLAIN).test(error.message)) throw error;
    }
    const text = material(key);
    if (text !== undefined && !wrapped && (typeof text === "string" ? text : Buffer.from(text).toString("latin1")).includes(PEM_MARKER)) {
      throw new Error(NOT_A_KEY);
    }
    return secretOf(key);
  };

  // Node's validateAsymmetricKey: the algorithm must suit the key type, an EC key's curve and an
  // RSA-PSS key's parameters.
  const KEY_ALGS = { ec: ["ES256", "ES384", "ES512"], rsa: ["RS256", "PS256", "RS384", "PS384", "RS512", "PS512"], "rsa-pss": ["PS256", "PS384", "PS512"] };
  const CURVES = { ES256: "prime256v1", ES384: "secp384r1", ES512: "secp521r1" };
  const checkAsymmetric = (alg, key) => {
    if (!key.kind) return;
    const allowed = KEY_ALGS[key.kind];
    if (!allowed) throw new Error(`Unknown key type "${key.kind}".`);
    if (!allowed.includes(alg)) throw new Error(`"alg" parameter for "${key.kind}" key type must be one of: ${allowed.join(", ")}.`);
    if (key.kind === "ec" && key.curve !== CURVES[alg]) throw new Error(`"alg" parameter "${alg}" requires curve "${CURVES[alg]}".`);
    if (key.kind === "rsa-pss") {
      const length = parseInt(alg.slice(-3), 10), { hashAlgorithm, mgf1HashAlgorithm, saltLength } = key.pss;
      if (hashAlgorithm !== `sha${length}` || mgf1HashAlgorithm !== hashAlgorithm) {
        throw new Error(`Invalid key for this operation, its RSA-PSS parameters do not meet the requirements of "alg" ${alg}.`);
      }
      if (saltLength !== undefined && saltLength > length >> 3) {
        throw new Error(`Invalid key for this operation, its RSA-PSS parameter saltLength does not meet the requirements of "alg" ${alg}.`);
      }
    }
  };

  const CLAIM_OPTIONS = { audience: "aud", issuer: "iss", subject: "sub", jwtid: "jti" };
  const OBJECT_ONLY = ["expiresIn", "notBefore", "noTimestamp", "audience", "issuer", "subject", "jwtid"];

  // Checks run in jsonwebtoken's order so the first failure carries its message. A callback gets
  // failures at once and the token on a microtask, as the library's signing stream does.
  function sign(payload, secretOrPrivateKey, options, callback) {
    if (typeof options === "function") {
      callback = options;
      options = {};
    } else {
      options = options || {};
    }
    // A Buffer is raw payload bytes, not claims. The `_isBuffer` probe keeps the bundle unloaded
    // for every other payload; a Buffer in hand means it is loaded already.
    const isBufferPayload = payload != null && payload._isBuffer === true && Buffer.isBuffer(payload);
    const isObjectPayload = typeof payload === "object" && !isBufferPayload;
    const header = Object.assign({ alg: options.algorithm || "HS256", typ: isObjectPayload ? "JWT" : undefined, kid: options.keyid }, options.header);
    const failure = (err) => {
      if (callback) return callback(err);
      throw err;
    };

    if (header.alg === "none") return failure(new JsonWebTokenError(`${header.alg} is not supported by Apinata`));
    if (!secretOrPrivateKey) return failure(new Error("secretOrPrivateKey must have a value"));
    let key;
    try {
      key = readKey(secretOrPrivateKey, "private");
    } catch (error) {
      return failure(error);
    }
    if (!key) return failure(new Error("secretOrPrivateKey is not valid key material"));
    if (header.alg.startsWith("HS") && key.type !== "secret") {
      return failure(new Error(`secretOrPrivateKey must be a symmetric key when using ${header.alg}`));
    } else if (/^(?:RS|PS|ES)/.test(header.alg) && key.type !== "private") {
      return failure(new Error(`secretOrPrivateKey must be an asymmetric key when using ${header.alg}`));
    }
    const sizeMessage = `secretOrPrivateKey has a minimum key size of 2048 bits for ${header.alg}`;
    if (!options.allowInsecureKeySizes && /^(?:RS|PS)/.test(header.alg) && key.bits < 2048) return failure(new Error(sizeMessage));

    if (typeof payload === "undefined") return failure(new Error("payload is required"));
    if (isObjectPayload) {
      try {
        validate(CLAIMS, true, payload, "payload");
      } catch (error) {
        return failure(error);
      }
      if (!options.mutatePayload) payload = Object.assign({}, payload);
    } else {
      const invalid = OBJECT_ONLY.filter((opt) => typeof options[opt] !== "undefined");
      if (invalid.length > 0) return failure(new Error(`invalid ${invalid.join(",")} option for ${typeof payload} payload`));
    }
    if (typeof payload.exp !== "undefined" && typeof options.expiresIn !== "undefined") {
      return failure(new Error('Bad "options.expiresIn" option the payload already has an "exp" property.'));
    }
    if (typeof payload.nbf !== "undefined" && typeof options.notBefore !== "undefined") {
      return failure(new Error('Bad "options.notBefore" option the payload already has an "nbf" property.'));
    }
    try {
      validate(SIGN_OPTIONS, false, options, "options");
      if (!options.allowInvalidAsymmetricKeyTypes) checkAsymmetric(header.alg, key);
    } catch (error) {
      return failure(error);
    }

    const timestamp = payload.iat || Math.floor(Date.now() / 1000);
    if (options.noTimestamp) delete payload.iat;
    else if (isObjectPayload) payload.iat = timestamp;
    const eg = ' eg: "1d", "20h", 60';
    if (typeof options.notBefore !== "undefined") {
      payload.nbf = timespan(options.notBefore, timestamp);
      if (typeof payload.nbf === "undefined") return failure(new Error(timespanMessage("notBefore") + eg));
    }
    if (typeof options.expiresIn !== "undefined" && isObjectPayload) {
      payload.exp = timespan(options.expiresIn, timestamp);
      if (typeof payload.exp === "undefined") return failure(new Error(timespanMessage("expiresIn") + eg));
    }
    for (const [option, claim] of Object.entries(CLAIM_OPTIONS)) {
      if (typeof options[option] === "undefined") continue;
      if (typeof payload[claim] !== "undefined") {
        return failure(new Error(`Bad "options.${option}" option. The payload already has an "${claim}" property.`));
      }
      payload[claim] = options[option];
    }

    let token;
    try {
      const text = typeof payload === "string" || typeof payload === "number" || isBufferPayload ? payload.toString() : JSON.stringify(payload);
      token = call("sign", header.alg, JSON.stringify(header), text, key.bytes);
      // The library's check on the result; only a non-RSA key let through by option can reach it.
      if (!options.allowInsecureKeySizes && /^(?:RS|PS)/.test(header.alg) && token.length < 256) throw new Error(sizeMessage);
    } catch (error) {
      if (!callback) throw error;
      Promise.resolve().then(() => callback(error));
      return undefined;
    }
    if (!callback) return token;
    Promise.resolve().then(() => callback(null, token));
    return undefined;
  }

  // jws.decode then jsonwebtoken's decode: null for a malformed token or header, and a payload
  // parsed as JSON when the header says JWT (which may throw) or when it happens to parse.
  function decode(token, options) {
    options = options || {};
    const text = typeof token === "string" ? token : typeof token === "number" ? String(token) : JSON.stringify(token);
    const parts = typeof text === "string" ? call("decode", text) : null;
    if (!parts) return null;
    let header;
    try {
      header = JSON.parse(parts[0]);
    } catch (_) {
      return null;
    }
    if (!header) return null;
    let payload = parts[1];
    if (header.typ === "JWT" || options.json) payload = JSON.parse(payload);
    if (typeof payload === "string") {
      try {
        const obj = JSON.parse(payload);
        if (obj !== null && typeof obj === "object") payload = obj;
      } catch (_) {}
    }
    return options.complete === true ? { header, payload, signature: parts[2] } : payload;
  }

  const HS_ALGS = ["HS256", "HS384", "HS512"];
  const RSA_ALGS = ["RS256", "RS384", "RS512", "PS256", "PS384", "PS512"];
  const EC_ALGS = ["ES256", "ES384", "ES512"];
  const ES_BYTES = { ES256: 64, ES384: 96, ES512: 132 };

  // Follows jsonwebtoken's verify step by step; a callback is called synchronously with its result.
  function verify(jwtString, secretOrPublicKey, options, callback) {
    if (typeof options === "function" && !callback) {
      callback = options;
      options = {};
    }
    options = Object.assign({}, options || {});
    const done = callback || ((err, data) => {
      if (err) throw err;
      return data;
    });

    if (options.clockTimestamp && typeof options.clockTimestamp !== "number") return done(new JsonWebTokenError("clockTimestamp must be a number"));
    if (options.nonce !== undefined && (typeof options.nonce !== "string" || options.nonce.trim() === "")) {
      return done(new JsonWebTokenError("nonce must be a non-empty string"));
    }
    if (options.allowInvalidAsymmetricKeyTypes !== undefined && typeof options.allowInvalidAsymmetricKeyTypes !== "boolean") {
      return done(new JsonWebTokenError("allowInvalidAsymmetricKeyTypes must be a boolean"));
    }
    const clockTimestamp = options.clockTimestamp || Math.floor(Date.now() / 1000);
    if (!jwtString) return done(new JsonWebTokenError("jwt must be provided"));
    if (typeof jwtString !== "string") return done(new JsonWebTokenError("jwt must be a string"));
    const parts = jwtString.split(".");
    if (parts.length !== 3) return done(new JsonWebTokenError("jwt malformed"));
    let decoded;
    try {
      decoded = decode(jwtString, { complete: true });
    } catch (err) {
      return done(err);
    }
    if (!decoded) return done(new JsonWebTokenError("invalid token"));
    const header = decoded.header;

    let getSecret;
    if (typeof secretOrPublicKey === "function") {
      if (!callback) return done(new JsonWebTokenError("verify must be called asynchronous if secret or public key is provided as a callback"));
      getSecret = secretOrPublicKey;
    } else {
      getSecret = (_, secretCallback) => secretCallback(null, secretOrPublicKey);
    }

    return getSecret(header, (err, secretOrPublicKey) => {
      if (err) return done(new JsonWebTokenError("error in secret or public key callback: " + err.message));
      const hasSignature = parts[2].trim() !== "";
      if (!hasSignature && secretOrPublicKey) return done(new JsonWebTokenError("jwt signature is required"));
      if (hasSignature && !secretOrPublicKey) return done(new JsonWebTokenError("secret or public key must be provided"));
      if (!hasSignature && !options.algorithms) return done(new JsonWebTokenError('please specify "none" in "algorithms" to verify unsigned tokens'));

      let key = { type: null, bytes: "" };
      if (secretOrPublicKey != null) {
        try {
          key = readKey(secretOrPublicKey, "public");
        } catch (e) {
          return done(e);
        }
        if (!key) return done(new JsonWebTokenError("secretOrPublicKey is not valid key material"));
      }
      if (!options.algorithms) {
        options.algorithms = key.type === "secret" ? HS_ALGS : key.kind === "ec" ? EC_ALGS : RSA_ALGS;
      }
      if (options.algorithms.indexOf(header.alg) === -1) return done(new JsonWebTokenError("invalid algorithm"));
      if (header.alg.startsWith("HS") && key.type !== "secret") {
        return done(new JsonWebTokenError(`secretOrPublicKey must be a symmetric key when using ${header.alg}`));
      } else if (/^(?:RS|PS|ES)/.test(header.alg) && key.type !== "public") {
        return done(new JsonWebTokenError(`secretOrPublicKey must be an asymmetric key when using ${header.alg}`));
      }
      if (!options.allowInvalidAsymmetricKeyTypes) {
        try {
          checkAsymmetric(header.alg, key);
        } catch (e) {
          return done(e);
        }
      }

      const wanted = ES_BYTES[header.alg], saw = Math.floor(parts[2].length * 3 / 4);
      if (wanted && saw !== wanted) return done(new TypeError(`"${header.alg}" signatures must be "${wanted}" bytes, saw "${saw}"`));
      let valid;
      try {
        valid = call("verify", jwtString, header.alg, key.bytes);
      } catch (e) {
        return done(e);
      }
      if (!valid) return done(new JsonWebTokenError("invalid signature"));

      const payload = decoded.payload;
      const tolerance = options.clockTolerance || 0;
      if (typeof payload.nbf !== "undefined" && !options.ignoreNotBefore) {
        if (typeof payload.nbf !== "number") return done(new JsonWebTokenError("invalid nbf value"));
        if (payload.nbf > clockTimestamp + tolerance) return done(new NotBeforeError("jwt not active", new Date(payload.nbf * 1000)));
      }
      if (typeof payload.exp !== "undefined" && !options.ignoreExpiration) {
        if (typeof payload.exp !== "number") return done(new JsonWebTokenError("invalid exp value"));
        if (clockTimestamp >= payload.exp + tolerance) return done(new TokenExpiredError("jwt expired", new Date(payload.exp * 1000)));
      }
      if (options.audience) {
        const audiences = Array.isArray(options.audience) ? options.audience : [options.audience];
        const target = Array.isArray(payload.aud) ? payload.aud : [payload.aud];
        const match = target.some((t) => audiences.some((a) => (a instanceof RegExp ? a.test(t) : a === t)));
        if (!match) return done(new JsonWebTokenError("jwt audience invalid. expected: " + audiences.join(" or ")));
      }
      if (options.issuer) {
        const invalidIssuer = (typeof options.issuer === "string" && payload.iss !== options.issuer) ||
          (Array.isArray(options.issuer) && options.issuer.indexOf(payload.iss) === -1);
        if (invalidIssuer) return done(new JsonWebTokenError("jwt issuer invalid. expected: " + options.issuer));
      }
      if (options.subject && payload.sub !== options.subject) return done(new JsonWebTokenError("jwt subject invalid. expected: " + options.subject));
      if (options.jwtid && payload.jti !== options.jwtid) return done(new JsonWebTokenError("jwt jwtid invalid. expected: " + options.jwtid));
      if (options.nonce && payload.nonce !== options.nonce) return done(new JsonWebTokenError("jwt nonce invalid. expected: " + options.nonce));
      if (options.maxAge) {
        if (typeof payload.iat !== "number") return done(new JsonWebTokenError("iat required when maxAge is specified"));
        const maxAgeTimestamp = timespan(options.maxAge, payload.iat);
        if (typeof maxAgeTimestamp === "undefined") {
          return done(new JsonWebTokenError(timespanMessage("maxAge") + ' eg: "1d", "20h", 60'));
        }
        if (clockTimestamp >= maxAgeTimestamp + tolerance) return done(new TokenExpiredError("maxAge exceeded", new Date(maxAgeTimestamp * 1000)));
      }
      if (options.complete === true) return done(null, { header, payload, signature: decoded.signature });
      return done(null, payload);
    });
  }

  // The module is built once; the global getter settles to it on first read, as libs.js does.
  const jwt = { sign, verify, decode };
  const settle = (value) =>
    Object.defineProperty(globalThis, "jwt", { value, writable: true, configurable: true });
  Object.defineProperty(globalThis, "jwt", {
    configurable: true,
    get() { settle(jwt); return jwt; },
    set(value) { settle(value); },
  });

  const previous = globalThis.require;
  globalThis.require = (name) => (String(name) === "jsonwebtoken" ? jwt : previous(name));
})();
