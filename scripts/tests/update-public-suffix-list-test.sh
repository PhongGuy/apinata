#!/usr/bin/env bash
# Tests for scripts/update-public-suffix-list.sh that never reach the network: PSL_SOURCE
# feeds a local list, PSL_ROOT points at a scratch tree, and a curl on PATH records any call.
set -euo pipefail
SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/update-public-suffix-list.sh"
REPO="$(cd "$(dirname "$SCRIPT")/.." && pwd)"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }
LIST=crates/apinata-core/vendor/public_suffix_list.dat
ROW_START="| \`$LIST\` |"

mkdir -p "$WORK/bin"
printf '#!/bin/sh\ntouch "%s/curl-called"\nexit 1\n' "$WORK" > "$WORK/bin/curl"
chmod +x "$WORK/bin/curl"

# A scratch tree holding copies of the committed list and THIRD-PARTY.md.
tree() {
  install -D -m 644 "$REPO/$LIST" "$WORK/$1/$LIST"
  install -D -m 644 "$REPO/THIRD-PARTY.md" "$WORK/$1/THIRD-PARTY.md"
}

run() {
  local root=$1 source=$2
  PATH="$WORK/bin:$PATH" PSL_ROOT="$WORK/$root" PSL_SOURCE="$source" bash "$SCRIPT"
}

unchanged() {
  cmp -s "$REPO/$LIST" "$WORK/$1/$LIST" || fail "$1: the list changed"
  cmp -s "${2:-$REPO/THIRD-PARTY.md}" "$WORK/$1/THIRD-PARTY.md" || fail "$1: THIRD-PARTY.md changed"
}

test_a_new_list_updates_both_files() {
  tree ok
  local fake=$WORK/fake.dat
  printf '// VERSION: 2027-01-02_03-04-05_UTC\n// COMMIT: 0123456789abcdef0123456789abcdef01234567\ncom\n' > "$fake"
  for i in $(seq 1 400); do printf 'suffix%s.test\n' "$i" >> "$fake"; done
  run ok "$fake" > "$WORK/ok.out" || fail "the update failed: $(cat "$WORK/ok.out")"
  cmp -s "$fake" "$WORK/ok/$LIST" || fail "the list was not replaced"
  local bytes sha row
  bytes=$(wc -c < "$fake" | sed -e ':a' -e 's/\B[0-9]\{3\}\>/,&/' -e 'ta')
  sha=$(sha256sum "$fake" | cut -d' ' -f1)
  row=$(grep -F "$ROW_START" "$WORK/ok/THIRD-PARTY.md")
  [[ $row == *'VERSION `2027-01-02_03-04-05_UTC`'* ]] || fail "VERSION not updated: $row"
  [[ $row == *'COMMIT `0123456789abcdef0123456789abcdef01234567`'* ]] || fail "COMMIT not updated: $row"
  [[ $row == *"($bytes bytes)"* ]] || fail "byte count not $bytes: $row"
  [[ $row == *"\`$sha\` |" ]] || fail "SHA-256 not $sha: $row"
  # Every other line is byte for byte what it was.
  diff <(grep -vF "$ROW_START" "$REPO/THIRD-PARTY.md") <(grep -vF "$ROW_START" "$WORK/ok/THIRD-PARTY.md") \
    > /dev/null || fail "other THIRD-PARTY.md lines changed"
  [[ $(wc -l < "$REPO/THIRD-PARTY.md") == $(wc -l < "$WORK/ok/THIRD-PARTY.md") ]] || fail "line count changed"
}

test_a_list_without_a_version_changes_nothing() {
  tree noversion
  printf 'com\nnet\n' > "$WORK/bad.dat"
  if run noversion "$WORK/bad.dat" > /dev/null 2>&1; then fail "a list without VERSION was accepted"; fi
  unchanged noversion
}

test_a_missing_row_changes_nothing() {
  tree norow
  grep -vF "$ROW_START" "$REPO/THIRD-PARTY.md" > "$WORK/norow/THIRD-PARTY.md"
  command cp -f "$WORK/norow/THIRD-PARTY.md" "$WORK/norow.md"
  local err
  if err=$(run norow "$REPO/$LIST" 2>&1); then fail "a THIRD-PARTY.md without the row was accepted"; fi
  [[ $err == *"THIRD-PARTY.md"* ]] || fail "error was: $err"
  unchanged norow "$WORK/norow.md"
}

test_a_doubled_row_changes_nothing() {
  tree twice
  grep -F "$ROW_START" "$REPO/THIRD-PARTY.md" >> "$WORK/twice/THIRD-PARTY.md"
  command cp -f "$WORK/twice/THIRD-PARTY.md" "$WORK/twice.md"
  printf '// VERSION: 2027-01-02_03-04-05_UTC\n// COMMIT: 0123456789abcdef0123456789abcdef01234567\ncom\n' > "$WORK/twice.dat"
  if run twice "$WORK/twice.dat" > /dev/null 2>&1; then fail "a doubled row was accepted"; fi
  unchanged twice "$WORK/twice.md"
}

test_a_list_without_a_commit_changes_nothing() {
  tree nocommit
  printf '// VERSION: 2027-01-02_03-04-05_UTC\ncom\n' > "$WORK/nocommit.dat"
  if run nocommit "$WORK/nocommit.dat" > /dev/null 2>&1; then fail "a list without COMMIT was accepted"; fi
  unchanged nocommit
}

# Values awk would expand (& and backslashes) or that are not the upstream shape are refused.
test_bad_header_values_change_nothing() {
  local good_v=2027-01-02_03-04-05_UTC good_c=0123456789abcdef0123456789abcdef01234567 i=0 pair
  for pair in "2027&01|$good_c" '2027\\x|'"$good_c" "$good_v|0123456789ABCDEF0123456789ABCDEF01234567" \
    "$good_v|abc" "$good_v|${good_c}&"; do
    i=$((i + 1))
    tree "bad$i"
    printf '// VERSION: %s\n// COMMIT: %s\ncom\n' "${pair%%|*}" "${pair#*|}" > "$WORK/bad$i.dat"
    if run "bad$i" "$WORK/bad$i.dat" > /dev/null 2>&1; then fail "header values $pair were accepted"; fi
    unchanged "bad$i"
  done
}

test_a_new_list_updates_both_files
test_a_list_without_a_commit_changes_nothing
test_bad_header_values_change_nothing
test_a_list_without_a_version_changes_nothing
test_a_missing_row_changes_nothing
test_a_doubled_row_changes_nothing
[[ ! -e $WORK/curl-called ]] || fail "curl was called"
[[ $FAILS == 0 ]] || { echo "$FAILS failure(s)" >&2; exit 1; }
echo "update-public-suffix-list: all tests passed"
