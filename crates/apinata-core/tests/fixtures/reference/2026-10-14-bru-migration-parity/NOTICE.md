# Notice

Source: https://github.com/usebruno/bruno at commit `1301ea17` (`1301ea1787f66aa8cf29cf6dec86ee3bc9eb4764`).

- `sources/unit/<group>/` holds `.bru` files from `packages/bruno-lang/v2/tests/`, unchanged, at the same relative paths: `custom-methods/fixtures/*.bru`, `fixtures/request.bru`, `fixtures/annotations.bru`, `fixtures/collection.bru`, `settings/fixtures/*.bru` and `examples/fixtures/bru/*.bru`.
- `sources/collections/bruno-tests/` is `packages/bruno-tests/collection/` and `sources/collections/response-examples/` is `tests/response-examples/fixtures/collection/`, every file, unchanged.
- `parsed/` and `expected/` are the reference's output over `sources/`: its `.bru` parsers and its on-disk YAML migration.
- `bundle.mjs` bundles `packages/bruno-lang/v2/src/`, `packages/bruno-electron/src/ipc/yml-migration.js`, `packages/bruno-filestore/src/`, `packages/bruno-common/src/` and `packages/bruno-requests/src/utils/proxy-util.ts` from that checkout; nothing from it is copied here.

The license (`license.md`) follows.

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