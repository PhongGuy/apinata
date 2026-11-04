#!/usr/bin/env bash
# Tests for scripts/readme-images-check.sh, run against throwaway repository roots.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SCRIPT=$ROOT/scripts/readme-images-check.sh
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

# A fresh root whose README is the first argument.
make_root() {
  local r=$WORK/$1
  mkdir -p "$r"
  printf '%s\n' "$2" >"$r/README.md"
  echo "$r"
}

test_skips_while_the_screenshot_directory_is_absent() {
  local r out
  r=$(make_root skip '![hero](assets/screenshots/hero.png)')
  out=$(bash "$SCRIPT" "$r" 2>&1) || fail "absent directory must exit 0"
  [[ $out == *"skipped: assets/screenshots/ not present"* ]] || fail "skip message: $out"
}

test_passes_when_every_image_exists() {
  local r
  r=$(make_root ok '![a](assets/screenshots/a.png) and <img src="assets/screenshots/b.png" width="300"> ![x](https://example.com/x.png)')
  mkdir -p "$r/assets/screenshots"
  touch "$r/assets/screenshots/a.png" "$r/assets/screenshots/b.png"
  bash "$SCRIPT" "$r" >/dev/null 2>&1 || fail "all present must pass"
}

test_fails_naming_each_missing_image() {
  local r out
  r=$(make_root miss '![a](assets/screenshots/a.png "title")
<img alt="b" src='"'"'assets/screenshots/b.png'"'"'>')
  mkdir -p "$r/assets/screenshots"
  touch "$r/assets/screenshots/a.png"
  if out=$(bash "$SCRIPT" "$r" 2>&1); then fail "missing image passed"; fi
  [[ $out == *"assets/screenshots/b.png"* && $out != *"a.png"* ]] || fail "listing: $out"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "readme-images-check tests passed"
