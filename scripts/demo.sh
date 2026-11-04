#!/usr/bin/env bash
# Opens the committed demo collection in the app with a throwaway profile:
#   bash scripts/demo.sh
# Each run copies examples/demo fresh into ${XDG_CACHE_HOME:-~/.cache}/apinata-demo, so edits
# made in the app never reach the repo. DEMO_DRY_RUN=1 prints the launch line instead.
# Must stay bash 3.2 safe, since macOS ships /bin/bash 3.2.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

die() { echo "demo: $*" >&2; exit 1; }

# XDG requires an absolute cache home; per the XDG spec, a relative one is ignored.
cache=${XDG_CACHE_HOME:-}
case $cache in
  /*) ;;
  *) cache=${HOME:?HOME is not set}/.cache ;;
esac
base=$cache/apinata-demo
# The copy below deletes $base/collection, so refuse anything that is not a real absolute path.
case $base in
  /*/apinata-demo) ;;
  *) die "refusing demo directory $base" ;;
esac

rm -rf "$base/collection"
mkdir -p "$base"
cp -R "$ROOT/examples/demo" "$base/collection"
mkdir -p "$base/profile"

# Build and launch from the repo, so a relative CARGO_TARGET_DIR means the same dir to both.
cd "$ROOT"
app=${CARGO_TARGET_DIR:-$ROOT/target}/release/apinata-app
if [[ -n ${DEMO_DRY_RUN:-} ]]; then
  echo "APINATA_PROFILE_DIR=$base/profile $app $base/collection"
  exit 0
fi

cargo build --release -p apinata-app
APINATA_PROFILE_DIR=$base/profile exec "$app" "$base/collection"
