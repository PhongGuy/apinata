/*!
 * @usebruno/query 0.1.0, packages/bruno-query/src/index.ts of usebruno/bruno at 1301ea17.
 * MIT License; see licenses/bruno-MIT.txt. Changes: the TypeScript annotations are removed, and
 * `export function get` is a function expression that the file returns.
 */
(function () {
  function normalize(value) {
    if (!Array.isArray(value)) return value;

    const values = [];

    value.forEach((item) => {
      const value = normalize(item);
      if (value != null) {
        values.push(...(Array.isArray(value) ? value : [value]));
      }
    });

    return values.length ? values : undefined;
  }

  function getValue(source, prop, deep = false) {
    if (typeof source !== 'object') return;

    let value;

    if (Array.isArray(source)) {
      value = source.map((item) => getValue(item, prop, deep));
    } else {
      value = source[prop];
      if (deep) {
        value = [value];
        for (const [key, item] of Object.entries(source)) {
          if (key !== prop && typeof item === 'object') {
            value.push(getValue(source[key], prop, deep));
          }
        }
      }
    }

    return normalize(value);
  }

  function objectPredicate(obj) {
    return (item) => {
      for (const [key, value] of Object.entries(obj)) {
        if (item[key] !== value) return false;
      }
      return true;
    };
  }

  function filterOrMap(source, funOrObj) {
    const fun = typeof funOrObj === 'object' ? objectPredicate(funOrObj) : funOrObj;
    const isArray = Array.isArray(source);
    const list = isArray ? source : [source];
    const result = [];
    for (const item of list) {
      if (item == null) continue;
      const value = fun(item);
      if (value === true) {
        result.push(item); // predicate
      } else if (value != null && value !== false) {
        result.push(value); // mapper
      }
    }
    return normalize(isArray ? result : result[0]);
  }

  function get(source, path, ...fns) {
    const paths = path
      .replace(/\s+/g, '')
      .split(/(\.{1,2}|\[\?\]|\[\d+\])/g) // ["..", "items", "[?]", ".", "amount", "[0]" ]
      .filter((s) => s.length > 0)
      .map((str) => {
        str = str.replace(/\[|\]/g, '');
        const index = parseInt(str);
        return isNaN(index) ? str : index;
      });

    let index = 0,
      lookbehind = '',
      funIndex = 0;

    while (source != null && index < paths.length) {
      const token = paths[index++];

      switch (true) {
        case token === '..':
        case token === '.':
          break;
        case token === '?':
          const fun = fns[funIndex++];
          if (fun == null) throw new Error(`missing function for ${lookbehind}`);
          source = filterOrMap(source, fun);
          break;
        case typeof token === 'number':
          source = normalize(source[token]);
          break;
        default:
          source = getValue(source, token, lookbehind === '..');
      }

      lookbehind = token;
    }

    return source;
  }

  return get;
})()
