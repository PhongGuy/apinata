#!/usr/bin/env bash
# Tests for scripts/actionlint.sh that need no network: a tampered download is refused.
set -euo pipefail
SCRIPT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/actionlint.sh
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

test_a_tampered_download_is_refused_and_not_cached() {
  local v=1.7.12 asset
  case $(uname -m) in x86_64) asset=linux_amd64 ;; aarch64) asset=linux_arm64 ;; *) return 0 ;; esac
  mkdir -p "$WORK/mirror/v$v" "$WORK/bin"
  printf '#!/bin/sh\necho tampered\n' >"$WORK/bin/actionlint"
  tar -czf "$WORK/mirror/v$v/actionlint_${v}_$asset.tar.gz" -C "$WORK/bin" actionlint
  local out
  if out=$(XDG_CACHE_HOME=$WORK/cache ACTIONLINT_BASE_URL=file://$WORK/mirror "$SCRIPT" 2>&1); then
    fail "tampered download ran: $out"
  fi
  [[ $out == *"sha256 mismatch"* ]] || fail "message: $out"
  [[ -z $(find "$WORK/cache" -type f 2>/dev/null) ]] || fail "tampered download left in the cache"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "actionlint tests passed"
