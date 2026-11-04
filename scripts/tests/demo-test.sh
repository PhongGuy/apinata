#!/usr/bin/env bash
# Tests for scripts/demo.sh: dry runs only, with HOME and the cache inside this suite's own
# temporary directory, so no app is launched and the real demo cache is never touched.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT=$ROOT/scripts/demo.sh
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

export HOME=$WORK/home DEMO_DRY_RUN=1
unset XDG_CACHE_HOME CARGO_TARGET_DIR
mkdir -p "$HOME"

test_dry_run_copies_the_demo_and_prints_the_isolated_launch() {
  local cache=$WORK/cache out
  out=$(XDG_CACHE_HOME=$cache CARGO_TARGET_DIR=$WORK/t DEMO_DRY_RUN=1 bash "$SCRIPT") || fail "dry run failed"
  diff -r "$ROOT/examples/demo" "$cache/apinata-demo/collection" >/dev/null || fail "copy differs from examples/demo"
  [[ $out == *"APINATA_PROFILE_DIR=$cache/apinata-demo/profile $WORK/t/release/apinata-app $cache/apinata-demo/collection"* ]] || fail "launch line: $out"
  [[ -d $cache/apinata-demo/profile ]] || fail "profile dir missing"
}

test_a_second_run_drops_stale_files() {
  local cache=$WORK/cache2
  XDG_CACHE_HOME=$cache DEMO_DRY_RUN=1 bash "$SCRIPT" >/dev/null || fail "first run failed"
  touch "$cache/apinata-demo/collection/stale.yml" || { fail "first run left no collection"; return 0; }
  XDG_CACHE_HOME=$cache DEMO_DRY_RUN=1 bash "$SCRIPT" >/dev/null || fail "second run failed"
  [[ ! -e $cache/apinata-demo/collection/stale.yml ]] || fail "stale file kept"
}

test_a_relative_xdg_cache_home_falls_back_to_home() {
  XDG_CACHE_HOME=rel HOME=$WORK/h DEMO_DRY_RUN=1 bash "$SCRIPT" >/dev/null || fail "run failed"
  [[ -d $WORK/h/.cache/apinata-demo/collection ]] || fail "did not use HOME/.cache"
}

test_the_repo_stays_clean() {
  [[ -z $(git -C "$ROOT" status --porcelain -- examples) ]] || fail "examples/ changed"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "demo tests passed"
