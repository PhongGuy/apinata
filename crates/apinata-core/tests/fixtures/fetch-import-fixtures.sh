#!/usr/bin/env bash
# Fetch the pinned importer fixtures into the directory given; see each NOTICE.md.
set -euo pipefail
out=$1
get() { # repo commit path destination
  mkdir -p "$(dirname "$out/$4")"
  curl -fsSL -A apinata-research -o "$out/$4" "https://raw.githubusercontent.com/$1/$2/$3"
}
OAI=69d8b7953c3259e243cf746388a0951b89649763   # OAI/OpenAPI-Specification tag 3.1.1
PET=d57941e8fe959e508796b27469b1e8bba73392dc   # swagger-api/swagger-petstore
BRU=1301ea1787f66aa8cf29cf6dec86ee3bc9eb4764   # usebruno/bruno
INS=c5e1798e4e19777c5b766c48be0af6a256070230   # Kong/insomnia core@13.3.1
HOP=63273f8e599e73f0fb38c0739314e8d46b6bc83d   # hoppscotch/hoppscotch 2026.9.0
get OAI/OpenAPI-Specification $OAI LICENSE openapi/oai/LICENSE
for f in v3.0/api-with-examples.yaml v3.0/callback-example.yaml v3.0/link-example.yaml v3.0/petstore-expanded.yaml \
         v3.0/petstore.yaml v3.0/petstore.json v3.0/uspto.yaml v3.1/non-oauth-scopes.yaml v3.1/webhook-example.yaml \
         v2.0/json/petstore.json v2.0/json/petstore-expanded.json v2.0/json/api-with-examples.json v2.0/json/uber.json \
         v2.0/yaml/petstore-minimal.yaml; do
  get OAI/OpenAPI-Specification $OAI "examples/$f" "openapi/oai/$f"
done
get swagger-api/swagger-petstore $PET LICENSE openapi/petstore/LICENSE
get swagger-api/swagger-petstore $PET src/main/resources/openapi.yaml openapi/petstore/openapi.yaml
for f in openapi-broken.json openapi-comprehensive.yaml openapi-duplicate-operation-name.yaml openapi-empty.yaml \
         openapi-invalid-extension.txt openapi-invalid-version.yaml openapi-malformed.yaml openapi-missing-info.yaml \
         openapi-multifile-endpoint.yaml openapi-multifile.yaml openapi-newline-in-operation-name.yaml \
         openapi-path-grouping.json openapi-simple.json openapi-with-examples.yaml openapi-with-security-schemes.json \
         openapi-without-security-schemes.json; do
  get usebruno/bruno $BRU "tests/import/openapi/fixtures/$f" "openapi/bruno/$f"
done
for f in insomnia-malformed.json insomnia-v4-with-envs.json insomnia-v4.json insomnia-v5-dates.yaml \
         insomnia-v5-invalid-missing-collection.yaml insomnia-v5-with-envs.yaml insomnia-v5.yaml; do
  get usebruno/bruno $BRU "tests/import/insomnia/fixtures/$f" "insomnia/bruno/$f"
done
get usebruno/bruno $BRU license.md openapi/bruno/LICENSE.md
cp "$out/openapi/bruno/LICENSE.md" "$out/insomnia/bruno/LICENSE.md"
S=packages/insomnia-smoke-test/fixtures
get Kong/insomnia $INS LICENSE insomnia/kong/LICENSE
for f in insomnia4.yaml import/multiple-workspaces.yaml environments.yaml environment-cascade-collection.yaml \
         auth-types.yaml template-tag-collection.yaml vault-environment.yaml unit-test.yaml global-environment.yaml; do
  get Kong/insomnia $INS "$S/$f" "insomnia/kong/$(basename "$f")"
done
get Kong/insomnia $INS packages/insomnia/src/plugins/context/__fixtures__/basic-import.json insomnia/kong/basic-import.json
for f in openapi3.yaml swagger2.yaml; do get Kong/insomnia $INS "$S/$f" "openapi/insomnia/$f"; done
cp "$out/insomnia/kong/LICENSE" "$out/openapi/insomnia/LICENSE"
for f in deep-input.json form-data-input.json minimal-input.json no-requests-input.json shallow-input.json; do
  get Kong/insomnia $INS "packages/insomnia/src/main/importers/importers/fixtures/har/$f" "har/kong/$f"
done
cp "$out/insomnia/kong/LICENSE" "$out/har/kong/LICENSE"
H=packages/hoppscotch-cli/src/__tests__/e2e/fixtures
get hoppscotch/hoppscotch $HOP LICENSE hoppscotch/LICENSE
for kind in collections environments; do
  curl -fsSL -A apinata-research "https://api.github.com/repos/hoppscotch/hoppscotch/contents/$H/$kind?ref=$HOP" \
    | python3 -c 'import json,sys; print("\n".join(x["name"] for x in json.load(sys.stdin) if not x["name"].endswith(".csv")))' \
    | while read -r f; do get hoppscotch/hoppscotch $HOP "$H/$kind/$f" "hoppscotch/$kind/$f"; done
done
