# Vendored Bruno Postman fixtures

Source: https://github.com/usebruno/bruno at commit `1301ea17`. Files are copied unmodified; only some are renamed.

| File | Source path |
| --- | --- |
| `collections/postman-*.json` | `tests/import/postman/fixtures/` (same names) |
| `collections/sample-postman.json` | `tests/import/test-data/sample-postman.json` |
| `collections/round-trip-<name>.json` | `packages/bruno-converters/tests/postman/round-trip/fixtures/<Name>.postman_collection.json`, lower-cased with spaces as `-` |
| `environments/postman-env.json`, `environments/postman-env-2.json` | `tests/environments/import-environment/multi-format-import/fixtures/` |
| `environments/postman-env-duplicate-a.json`, `environments/postman-env-duplicate-b.json` | `tests/environments/import-environment/name-conflicts/fixtures/` |
| `environments/postman-env-mixed-enabled.json` | `tests/environments/import-environment/fixtures/` |
| `environments/postman-collection-env.json` | `tests/environments/import-environment/fixtures/collection-env.json` |
| `environments/postman-global-env.json` | `tests/environments/import-environment/fixtures/global-env.json` |

`collections/apinata-edge-cases.json` and `environments/apinata-env-edge-cases.json` are Apinata-authored and not from Bruno. So is `own/scripts-subset.json`.

Bruno's license (`license.md`) follows.

MIT License

Copyright (c) 2022 Anoop M D, Anusree P S and Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
