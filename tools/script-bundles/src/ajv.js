// Entry for the ajv bundle: Ajv and ajv-formats share one copy of ajv, so
// addFormats(ajv) accepts an instance built from the exported class.
const Ajv = require("ajv");
const addFormats = require("ajv-formats");
module.exports = { Ajv: Ajv.default || Ajv, addFormats: addFormats.default || addFormats };
