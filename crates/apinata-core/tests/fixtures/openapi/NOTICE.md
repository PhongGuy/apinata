# Vendored OpenAPI fixtures

Sources, one per subdirectory. Files are copied unmodified; `fetch-import-fixtures.sh` in the parent directory pins every download.

- `apinata/`: authored for Apinata, not downloaded, licensed MIT OR Apache-2.0.
- `oai/`: https://github.com/OAI/OpenAPI-Specification at commit `69d8b7953c3259e243cf746388a0951b89649763` (tag `3.1.1`), licensed Apache-2.0.
- `petstore/`: https://github.com/swagger-api/swagger-petstore at commit `d57941e8fe959e508796b27469b1e8bba73392dc`, licensed Apache-2.0.
- `bruno/`: https://github.com/usebruno/bruno at commit `1301ea1787f66aa8cf29cf6dec86ee3bc9eb4764`, licensed MIT.
- `insomnia/`: https://github.com/Kong/insomnia at commit `c5e1798e4e19777c5b766c48be0af6a256070230` (tag `core@13.3.1`), licensed Apache-2.0.

| File | Source path |
| --- | --- |
| `oai/v2.0/`, `oai/v3.0/`, `oai/v3.1/` | `examples/v2.0/`, `examples/v3.0/`, `examples/v3.1/` (same names) |
| `petstore/openapi.yaml` | `src/main/resources/openapi.yaml` |
| `bruno/*` | `tests/import/openapi/fixtures/` (same names) |
| `insomnia/openapi3.yaml`, `insomnia/swagger2.yaml` | `packages/insomnia-smoke-test/fixtures/` (same names) |

The licence texts are in `oai/LICENSE`, `petstore/LICENSE`, `bruno/LICENSE.md` and `insomnia/LICENSE`.
