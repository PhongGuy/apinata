# Notice

Source: https://github.com/usebruno/bruno at commit `1301ea17`.

- `test-key.pem` is `TEST_RSA_PRIVATE_KEY` from `packages/bruno-tests/src/auth/oauth1/index.js`, copied unmodified; `test-key-pkcs1.pem` is the same key converted with `openssl rsa -traditional`. Both are test keys that protect nothing.
- `vectors.out` is the output of the reference signer `packages/bruno-requests/src/auth/oauth1-request-authorization.ts`, which `prepare.sh` copies (not committed) with one import inlined from `packages/bruno-common/src/utils/form-data.ts`.

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
