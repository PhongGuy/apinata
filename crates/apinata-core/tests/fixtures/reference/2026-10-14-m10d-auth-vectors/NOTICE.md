# Notice

Source: https://github.com/usebruno/bruno at commit `1301ea17`.

- `vectors.json` is the output of the reference's own code, bundled by `bundle.mjs` from the checkout and not committed: `packages/bruno-requests/src/auth/edgegrid-helper.js` with `packages/bruno-common/src/utils/` (including `edgegrid.ts`), `packages/bruno-electron/src/ipc/network/prepare-request.js` with `src/utils/collection.js` and `src/utils/form-data.js`, and the YAML reader `packages/bruno-filestore/src/formats/yml/`.
- `vectors.mjs` repeats a few lines of `packages/bruno-electron/src/ipc/network/index.js` (the post-interpolation body transforms in `runPreRequest`) and of `axios-instance.js` (the `axios.create` options), marked where they appear.
- The EdgeGrid credentials in `vectors.mjs` are made-up placeholders that protect nothing.

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
