// xml2js's async mode yields through setImmediate; the engine's setTimeout does that job here.
module.exports = { setImmediate: (fn, ...args) => setTimeout(() => fn(...args), 0) };
