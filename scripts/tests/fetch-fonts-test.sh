#!/usr/bin/env bash
# Tests for scripts/fetch-fonts.sh that need no network: the committed files pass --check,
# and a tampered or missing copy fails it.
set -euo pipefail
SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/fetch-fonts.sh"
REPO="$(cd "$(dirname "$SCRIPT")/.." && pwd)"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

copy_tree() {
  local f
  while read -r f; do install -D -m 644 "$REPO/$f" "$WORK/$1/$f"; done < <(bash "$SCRIPT" --list)
}

test_committed_files_pass() {
  bash "$SCRIPT" --check || fail "committed files fail --check"
  [[ $(bash "$SCRIPT" --list | wc -l) == 9 ]] || fail "--list does not name nine files"
}

test_tampered_font_fails() {
  copy_tree tampered
  printf 'X' | dd of="$WORK/tampered/crates/apinata-app/assets/fonts/Inter-Medium.ttf" bs=1 seek=100 conv=notrunc status=none
  local err
  if err=$(FETCH_FONTS_ROOT=$WORK/tampered bash "$SCRIPT" --check 2>&1); then fail "tampered font passed"; fi
  [[ $err == *"mismatch for crates/apinata-app/assets/fonts/Inter-Medium.ttf"* ]] || fail "error was: $err"
}

test_missing_licence_fails() {
  copy_tree missing
  rm "$WORK/missing/licenses/Lucide-ISC.txt"
  local err
  if err=$(FETCH_FONTS_ROOT=$WORK/missing bash "$SCRIPT" --check 2>&1); then fail "missing licence passed"; fi
  [[ $err == *"missing licenses/Lucide-ISC.txt"* ]] || fail "error was: $err"
}

test_unknown_argument_exits_2() {
  local code=0
  bash "$SCRIPT" --nope 2>/dev/null || code=$?
  [[ $code == 2 ]] || fail "exit $code, want 2"
}

test_committed_files_pass
test_tampered_font_fails
test_missing_licence_fails
test_unknown_argument_exits_2
[[ $FAILS == 0 ]] || { echo "$FAILS failure(s)" >&2; exit 1; }
echo "fetch-fonts: all tests passed"
