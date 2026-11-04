/*!
 * The `json`, `jsonSchema` and `jsonBody` chai assertions, adapted from usebruno/bruno at commit 1301ea17,
 * packages/bruno-js/src/sandbox/quickjs/shims/test.js. MIT License,
 * Copyright (c) 2022 Anoop M D, Anusree P S and Contributors; see licenses/bruno-MIT.txt.
 * Changes: wrapped as a function of chai, ajv is loaded on the first `jsonSchema` call through the
 * internal library loader, and the unsupported-version message names Apinata.
 */
(function (chai) {
  var proto = chai.Assertion.prototype;

  Object.defineProperty(proto, 'json', {
    get: function () {
      var obj = this._obj;
      var isJson = typeof obj === 'object' && obj !== null &&
        (Array.isArray(obj) || Object.prototype.toString.call(obj) === '[object Object]');
      this.assert(isJson, 'expected #{this} to be JSON', 'expected #{this} not to be JSON');
      return this;
    },
    configurable: true
  });

  // Validation runs over the bundled ajv (Draft-07) with ajv-formats. The default instance is built on
  // the first call and reused; options build a fresh one. Negation quirks follow the reference as is.
  var loadAjv = __apinata.ajv;
  var defaultAjv;
  var SUPPORTED_SCHEMA_VERSIONS = [
    'http://json-schema.org/draft-07/schema#',
    'http://json-schema.org/draft-07/schema'
  ];
  proto.jsonSchema = function (schema, ajvOptions) {
    var lib = loadAjv();
    var Ajv = lib.Ajv;
    var addFormats = lib.addFormats;
    if (schema && schema.$schema && !SUPPORTED_SCHEMA_VERSIONS.includes(schema.$schema)) {
      this.assert(
        false,
        'Unsupported JSON Schema version: "' + schema.$schema + '". Apinata currently only supports Draft-07 (http://json-schema.org/draft-07/schema#). Please update your schema to be Draft-07 compatible and remove the $schema property.',
        'Unsupported JSON Schema version: "' + schema.$schema + '".'
      );
    }
    var ajv;
    if (ajvOptions) {
      ajv = new Ajv(Object.assign({ allErrors: true }, ajvOptions));
      addFormats(ajv);
    } else {
      if (!defaultAjv) {
        defaultAjv = new Ajv({ allErrors: true });
        addFormats(defaultAjv);
      }
      ajv = defaultAjv;
    }
    var validate;
    try {
      validate = ajv.compile(schema);
    } catch (e) {
      this.assert(false, 'JSON schema compile error: ' + e.message, 'JSON schema compile error: ' + e.message);
    }
    var data = this._obj;
    var isValid = validate(data);

    var dataStr;
    try { dataStr = JSON.stringify(data); } catch (e) { dataStr = '[unserializable value]'; }
    this.assert(
      isValid,
      'expected ' + dataStr + ' to match JSON schema, validation errors: ' + (validate.errors ? JSON.stringify(validate.errors) : 'none'),
      'expected ' + dataStr + ' to not match JSON schema'
    );
    return this;
  };

  // Parse a property path into an array of keys.
  // Handles: dot notation (a.b), numeric brackets (a[0]), quoted brackets (a["b.c"], a['key']),
  // and combinations like data[0]["a.b"].name
  function parsePath(path) {
    var keys = [];
    var i = 0;
    while (i < path.length) {
      if (path[i] === '.') {
        i++;
      } else if (path[i] === '[') {
        i++;
        if (i < path.length && (path[i] === "'" || path[i] === '"')) {
          var quote = path[i];
          i++;
          var key = '';
          while (i < path.length && path[i] !== quote) {
            if (path[i] === '\\' && i + 1 < path.length && path[i + 1] === quote) {
              key += quote;
              i += 2;
            } else {
              key += path[i];
              i++;
            }
          }
          i++; // skip closing quote
          i++; // skip ']'
          keys.push(key);
        } else {
          var key = '';
          while (i < path.length && path[i] !== ']') {
            key += path[i];
            i++;
          }
          i++; // skip ']'
          keys.push(key);
        }
      } else {
        var key = '';
        while (i < path.length && path[i] !== '.' && path[i] !== '[') {
          key += path[i];
          i++;
        }
        keys.push(key);
      }
    }
    return keys;
  }

  function getNestedValue(obj, path) {
    var keys = parsePath(path);
    var current = obj;
    for (var i = 0; i < keys.length; i++) {
      var key = keys[i];
      if (current === null || current === undefined || !Object.prototype.hasOwnProperty.call(Object(current), key)) {
        return { found: false };
      }
      current = current[key];
    }
    return { found: true, value: current };
  }

  function deepEqual(a, b) {
    if (a === b) return true;
    if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    var keysA = Object.keys(a);
    var keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    for (var i = 0; i < keysA.length; i++) {
      if (!Object.prototype.hasOwnProperty.call(b, keysA[i]) || !deepEqual(a[keysA[i]], b[keysA[i]])) return false;
    }
    return true;
  }

  proto.jsonBody = function () {
    var obj = this._obj;
    var args = Array.prototype.slice.call(arguments);

    if (args.length === 0) {
      this.assert(
        typeof obj === 'object' && obj !== null,
        'expected value to be a JSON body (object or array)',
        'expected value not to be a JSON body'
      );
    } else if (args.length === 1 && typeof args[0] === 'object' && args[0] !== null) {
      this.assert(
        deepEqual(obj, args[0]),
        'expected body to deeply equal given object',
        'expected body to not deeply equal given object'
      );
    } else if (args.length === 1) {
      var result = getNestedValue(obj, String(args[0]));
      this.assert(
        result.found,
        "expected body to have nested property '" + args[0] + "'",
        "expected body to not have nested property '" + args[0] + "'"
      );
    } else {
      var result = getNestedValue(obj, String(args[0]));
      this.assert(
        result.found && deepEqual(result.value, args[1]),
        "expected body to have nested property '" + args[0] + "' equal to given value",
        "expected body to not have nested property '" + args[0] + "' equal to given value"
      );
    }
    return this;
  };
})
