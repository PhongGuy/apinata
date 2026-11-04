# Vendored Insomnia fixtures

Sources, one per subdirectory. Files are copied unmodified; `fetch-import-fixtures.sh` in the parent directory pins every download.

- `kong/`: https://github.com/Kong/insomnia at commit `c5e1798e4e19777c5b766c48be0af6a256070230` (tag `core@13.3.1`), licensed Apache-2.0.
- `bruno/`: https://github.com/usebruno/bruno at commit `1301ea1787f66aa8cf29cf6dec86ee3bc9eb4764`, licensed MIT.

| File | Source path |
| --- | --- |
| `kong/*.yaml` | `packages/insomnia-smoke-test/fixtures/` (same names), except `multiple-workspaces.yaml` from `packages/insomnia-smoke-test/fixtures/import/` |
| `kong/basic-import.json` | `packages/insomnia/src/plugins/context/__fixtures__/basic-import.json` |
| `bruno/*` | `tests/import/insomnia/fixtures/` (same names) |

The licence texts are in `kong/LICENSE` and `bruno/LICENSE.md`.
