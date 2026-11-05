#!/usr/bin/env bash
# Tests for the Makefile: each target prints exactly the commands contributors and CI rely on.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

# Dry-run make in an empty directory, so files named like targets can be planted there, and
# without cargo's or an outer make's environment, so the Makefile's own defaults apply.
dry() {
  env -u CARGO -u MAKEFLAGS -u MAKELEVEL -u MFLAGS \
    make --no-print-directory -n -f "$ROOT/Makefile" -C "$WORK" "$@"
}

expect() {
  local want=$1 got
  shift
  got=$(dry "$@" 2>&1) || { fail "make -n $* failed: $got"; return 0; }
  [[ $got == "$want" ]] || fail "make -n $* printed [$got], want [$want]"
}

GATE=$'cargo fmt --check\ncargo clippy --all-targets --all-features -- -Dwarnings\nenv -u WAYLAND_DISPLAY -u DISPLAY cargo test --workspace\nbash scripts/readme-images-check.sh'

test_check_runs_format_lint_and_tests_in_order() {
  expect "$GATE" check
}

test_single_targets_print_their_one_command() {
  expect 'cargo fmt' fmt
  expect 'cargo fmt --check' fmt-check
  expect 'cargo clippy --all-targets --all-features -- -Dwarnings' lint
  expect 'env -u WAYLAND_DISPLAY -u DISPLAY cargo test --workspace' test
  expect 'scripts/package-linux.sh' package
  expect 'bash scripts/release.sh package' package-release
  expect 'cargo clean' clean
  expect 'cargo xtask icons --check' icons-check
  expect 'cargo xtask check-windows --require' windows-check
  expect 'scripts/actionlint.sh' workflows-lint
  expect 'bash scripts/licenses.sh generate' licenses
  expect 'bash scripts/licenses.sh check' licenses-check
}

test_targets_run_even_when_a_file_has_their_name() {
  touch "$WORK/check" "$WORK/fmt-check" "$WORK/lint" "$WORK/test" "$WORK/package" "$WORK/clean" "$WORK/readme-images-check"
  expect "$GATE" check
  expect 'scripts/package-linux.sh' package
  expect 'cargo clean' clean
  rm -f "$WORK/check" "$WORK/fmt-check" "$WORK/lint" "$WORK/test" "$WORK/package" "$WORK/clean" "$WORK/readme-images-check"
}

test_bare_make_and_install_run_the_install_script() {
  expect 'bash scripts/install.sh install'
  expect 'bash scripts/install.sh install' PREFIX="$WORK/p"
  expect 'bash scripts/install.sh install' install
  expect 'bash scripts/install.sh uninstall' uninstall
}

test_demo_runs_the_demo_script() {
  expect 'bash scripts/demo.sh' demo
}

test_cargo_can_be_overridden() {
  expect '/opt/cargo clippy --all-targets --all-features -- -Dwarnings' CARGO=/opt/cargo lint
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "makefile tests passed"
