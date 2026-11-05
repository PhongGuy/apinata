#!/usr/bin/env bash
# Tests for the functions in scripts/release.sh.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../release.sh"
RELEASE_SH=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/release.sh

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

test_check_ref_accepts_only_the_matching_tag() {
  check_ref tag v1.2.3 1.2.3 false || fail "matching tag refused"
  if check_ref tag v1.2.4 1.2.3 false 2>/dev/null; then fail "mismatched tag accepted"; fi
  if check_ref tag 1.2.3 1.2.3 false 2>/dev/null; then fail "tag without v accepted"; fi
  check_ref tag v1.2.3 1.2.3 true || fail "matching tag refused on a dry run"
  if check_ref tag v0.0.0-test 1.2.3 true 2>/dev/null; then fail "mismatched tag accepted on a dry run"; fi
  local msg; msg=$(check_ref tag v9.9.9 1.2.3 false 2>&1) || true
  [[ $msg == *"v9.9.9"*"1.2.3"* ]] || fail "mismatch message: $msg"
}

test_check_ref_allows_a_branch_only_on_a_dry_run() {
  check_ref branch main 1.2.3 true || fail "dry run from a branch refused"
  if check_ref branch main 1.2.3 false 2>/dev/null; then fail "release from a branch accepted"; fi
  if check_ref branch main 1.2.3 "" 2>/dev/null; then fail "empty dry_run treated as a dry run"; fi
  if check_ref pull 1 1.2.3 true 2>/dev/null; then fail "unknown ref type accepted"; fi
}

test_select_targets_builds_the_matrix_in_a_fixed_order() {
  local linux='{"os":"ubuntu-24.04","target":"x86_64-unknown-linux-gnu"},{"os":"ubuntu-24.04-arm","target":"aarch64-unknown-linux-gnu"}'
  local windows='{"os":"windows-2025","target":"x86_64-pc-windows-msvc"}'
  local macos='{"os":"macos-15","target":"aarch64-apple-darwin"},{"os":"macos-15-intel","target":"x86_64-apple-darwin"}'
  [[ $(select_targets "" false) == "{\"include\":[$linux,$windows,$macos]}" ]] || fail "empty: $(select_targets "" false)"
  [[ $(select_targets "macos, linux" true) == "{\"include\":[$linux,$macos]}" ]] || fail "subset: $(select_targets "macos, linux" true)"
  [[ $(select_targets windows true) == "{\"include\":[$windows]}" ]] || fail "windows: $(select_targets windows true)"
  local arm='{"os":"macos-15","target":"aarch64-apple-darwin"}'
  [[ $(select_targets macos-arm64 true) == "{\"include\":[$arm]}" ]] || fail "macos-arm64: $(select_targets macos-arm64 true)"
  [[ $(select_targets macos-arm64,macos true) == "{\"include\":[$macos]}" ]] || fail "macos-arm64 with macos: $(select_targets macos-arm64,macos true)"
}

test_select_targets_refuses_unknown_and_partial_releases() {
  if select_targets "linux,beos" true 2>/dev/null; then fail "unknown platform accepted"; fi
  if select_targets "linux" false 2>/dev/null; then fail "a release without every platform accepted"; fi
  if select_targets "linux,windows,macos-arm64" false 2>/dev/null; then fail "a release with one macOS leg accepted"; fi
  select_targets "windows,macos,linux" false >/dev/null || fail "a full release refused"
}

test_guard_prints_the_version_and_matrix() {
  local out
  out=$(bash "$RELEASE_SH" guard branch main true windows) || { fail "guard failed: $out"; return 0; }
  [[ $(sed -n 1p <<<"$out") =~ ^version=[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "guard version line: $out"
  [[ $(sed -n 2p <<<"$out") == 'matrix={"include":[{"os":"windows-2025","target":"x86_64-pc-windows-msvc"}]}' ]] \
    || fail "guard matrix line: $out"
  if bash "$RELEASE_SH" guard tag v0.0.0-test false "" >/dev/null 2>&1; then fail "a mismatched tag passed the guard"; fi
}

test_sums_writes_checksums_for_a_directory() {
  mkdir -p "$WORK/s"
  printf a >"$WORK/s/apinata-1-x.tar.gz"
  bash "$RELEASE_SH" sums "$WORK/s" || fail "sums failed"
  (cd "$WORK/s" && sha256sum -c --quiet SHA256SUMS) || fail "SHA256SUMS does not verify"
}

# The layout a per-platform artifact download leaves: files one level down.
test_sums_hashes_files_in_platform_subdirectories() {
  local d=$WORK/nested
  mkdir -p "$d/linux" "$d/macos"
  printf a >"$d/linux/apinata-1-x86_64-unknown-linux-gnu"; printf b >"$d/linux/apinata-app_1_amd64.deb"
  printf c >"$d/macos/Apinata_1_aarch64.dmg"; printf d >"$d/linux/notes.txt"
  bash "$RELEASE_SH" sums "$d" || fail "sums failed on a nested tree"
  [[ $(cut -d' ' -f3 "$d/SHA256SUMS" | tr '\n' ' ') == "linux/apinata-1-x86_64-unknown-linux-gnu linux/apinata-app_1_amd64.deb macos/Apinata_1_aarch64.dmg " ]] \
    || fail "nested SHA256SUMS names: $(cut -d' ' -f3 "$d/SHA256SUMS" | tr '\n' ' ')"
  (cd "$d" && sha256sum -c --quiet SHA256SUMS) || fail "nested SHA256SUMS does not verify"
}

test_sums_fails_when_nothing_is_hashed() {
  mkdir -p "$WORK/none"
  touch "$WORK/none/notes.txt"
  if bash "$RELEASE_SH" sums "$WORK/none" 2>/dev/null; then fail "sums passed with no artifacts"; fi
}

# Set the named variables to VALUE, as a workflow step's env does with secrets.
set_vars() {
  local value=$1 v
  shift
  for v in "$@"; do export "$v=$value"; done
}

test_windows_signing_mode_needs_both_or_neither_secret() {
  set_vars "" WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD
  [[ $(windows_signing_mode) == unsigned ]] || fail "blank secrets not unsigned"
  set_vars "  " WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD
  [[ $(windows_signing_mode) == unsigned ]] || fail "whitespace secrets not unsigned"
  unset WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD
  [[ $(windows_signing_mode) == unsigned ]] || fail "unset secrets not unsigned"
  set_vars x WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD
  [[ $(windows_signing_mode) == signed ]] || fail "both secrets not signed"
  export WINDOWS_CERTIFICATE_PASSWORD=""
  local msg
  if msg=$(windows_signing_mode 2>&1); then fail "certificate without password accepted"; fi
  [[ $msg == *WINDOWS_CERTIFICATE_PASSWORD* && $msg != *"WINDOWS_CERTIFICATE "* ]] || fail "message: $msg"
  unset WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD
}

test_prepare_windows_signing_unsets_blank_secrets_when_unsigned() {
  set_vars "" WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD APINATA_WINDOWS_SIGN_COMMAND
  prepare_windows_signing unsigned "$WORK/keys-u"
  local v
  for v in WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD APINATA_WINDOWS_SIGN_COMMAND; do
    [[ -z ${!v+set} ]] || fail "$v still set after an unsigned prepare"
  done
  [[ ! -e $WORK/keys-u ]] || fail "key directory created for an unsigned build"
}

test_prepare_windows_signing_writes_a_private_certificate_and_sign_command() {
  printf 'pfx bytes' >"$WORK/cert.pfx"
  export WINDOWS_CERTIFICATE=$(base64 -w0 "$WORK/cert.pfx") WINDOWS_CERTIFICATE_PASSWORD='pw with space'
  prepare_windows_signing signed "$WORK/keys-s"
  cmp -s "$WORK/keys-s/apinata.pfx" "$WORK/cert.pfx" || fail "certificate not decoded"
  [[ $(stat -c %a "$WORK/keys-s/apinata.pfx") == 600 ]] || fail "certificate mode $(stat -c %a "$WORK/keys-s/apinata.pfx")"
  [[ $(stat -c %a "$WORK/keys-s") == 700 ]] || fail "key dir mode $(stat -c %a "$WORK/keys-s")"
  [[ -z ${WINDOWS_CERTIFICATE+set} ]] || fail "certificate still in the environment"
  [[ ${APINATA_WINDOWS_SIGN_COMMAND:-} == "bash $WORK/keys-s/sign.sh %1" ]] || fail "sign command: ${APINATA_WINDOWS_SIGN_COMMAND:-}"
  # Match the password value and signtool's flag, not a substring a mktemp path can hold.
  [[ $APINATA_WINDOWS_SIGN_COMMAND != *"$WINDOWS_CERTIFICATE_PASSWORD"* && $APINATA_WINDOWS_SIGN_COMMAND != *" /p "* ]] \
    || fail "password in the sign command"

  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$@" >"%s/signtool.args"\n' "$WORK" >"$WORK/signtool"
  chmod +x "$WORK/signtool"
  SIGNTOOL=$WORK/signtool bash "$WORK/keys-s/sign.sh" 'C:\out\apinata-app.exe' || fail "sign.sh failed"
  local want; want=$(printf '%s\n' sign /fd SHA256 /f "$WORK/keys-s/apinata.pfx" /p 'pw with space' /d Apinata 'C:\out\apinata-app.exe')
  [[ $(cat "$WORK/signtool.args") == "$want" ]] || fail "signtool args: $(tr '\n' '|' <"$WORK/signtool.args")"
  unset WINDOWS_CERTIFICATE_PASSWORD APINATA_WINDOWS_SIGN_COMMAND
}

test_prepare_windows_signing_refuses_a_bad_certificate_or_a_spaced_path() {
  export WINDOWS_CERTIFICATE='not base64!' WINDOWS_CERTIFICATE_PASSWORD=pw
  if (prepare_windows_signing signed "$WORK/keys-bad") 2>/dev/null; then fail "undecodable certificate accepted"; fi
  export WINDOWS_CERTIFICATE=$(printf x | base64)
  if (prepare_windows_signing signed "$WORK/with space") 2>/dev/null; then fail "key dir with a space accepted"; fi
  if (SIGN_SHELL='C:/Program Files/Git/usr/bin/bash.exe' prepare_windows_signing signed "$WORK/keys-sp") 2>/dev/null; then
    fail "sign shell with a space accepted"
  fi
  unset WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD
}

test_mark_unsigned_renames_only_windows_artifacts() {
  local d=$WORK/marks
  mkdir -p "$d"
  touch "$d/apinata-app_1.0.0_x64-setup.exe" "$d/apinata-1.0.0-x86_64-pc-windows-msvc.zip" "$d/Apinata_1.0.0_aarch64.dmg"
  mark_unsigned "$d"
  local got; got=$(ls "$d" | LC_ALL=C sort | tr '\n' ' ')
  [[ $got == "Apinata_1.0.0_aarch64.dmg apinata-1.0.0-x86_64-pc-windows-msvc-unsigned.zip apinata-app_1.0.0_x64-setup-unsigned.exe " ]] \
    || fail "after mark_unsigned: $got"
}

test_macos_dmg_ad_hoc_signs_then_wraps_the_app() {
  local out=$WORK/mac
  mkdir -p "$out/Apinata.app/Contents"
  printf '#!/usr/bin/env bash\necho "codesign $*" >>"%s/mac.log"\n' "$WORK" >"$WORK/codesign"
  printf '#!/usr/bin/env bash\necho "hdiutil $*" >>"%s/mac.log"\ntouch "${@: -1}"\n' "$WORK" >"$WORK/hdiutil"
  chmod +x "$WORK/codesign" "$WORK/hdiutil"
  CODESIGN=$WORK/codesign HDIUTIL=$WORK/hdiutil macos_dmg "$out" 1.2.3 aarch64 || fail "macos_dmg failed"
  local want="codesign --force --deep --sign - $out/Apinata.app
codesign --verify --deep --strict $out/Apinata.app
hdiutil create -volname Apinata -srcfolder $out/Apinata.app -ov -format UDZO $out/Apinata_1.2.3_aarch64.dmg"
  [[ $(cat "$WORK/mac.log") == "$want" ]] || fail "macOS commands: $(cat "$WORK/mac.log")"
  [[ -f $out/Apinata_1.2.3_aarch64.dmg && ! -e $out/Apinata.app ]] || fail "dmg missing or app left: $(ls "$out")"
}

test_cli_zip_holds_the_exe_and_files() {
  printf 'MZ' >"$WORK/apinata.exe"; printf 'mit\n' >"$WORK/LICENSE-MIT"; printf 'readme\n' >"$WORK/README.md"
  mkdir -p "$WORK/licenses"; printf 'chai\n' >"$WORK/licenses/chai-MIT.txt"
  cli_zip "$WORK/apinata.exe" 9.9.9 x86_64-pc-windows-msvc "$WORK/zout" "$WORK/LICENSE-MIT" "$WORK/README.md" "$WORK/licenses"
  local z=$WORK/zout/apinata-9.9.9-x86_64-pc-windows-msvc.zip
  [[ -f $z ]] || { fail "zip not written"; return 0; }
  local listing; listing=$(unzip -Z1 "$z" | LC_ALL=C sort | tr '\n' ' ')
  local want="apinata-9.9.9-x86_64-pc-windows-msvc/ apinata-9.9.9-x86_64-pc-windows-msvc/LICENSE-MIT apinata-9.9.9-x86_64-pc-windows-msvc/README.md apinata-9.9.9-x86_64-pc-windows-msvc/apinata.exe apinata-9.9.9-x86_64-pc-windows-msvc/licenses/ apinata-9.9.9-x86_64-pc-windows-msvc/licenses/chai-MIT.txt "
  [[ $listing == "$want" ]] || fail "zip listing: $listing"
}

test_portable_realpath_matches_gnu_realpath() {
  local p
  for p in / "$WORK" "$WORK/a/../b" "$WORK/missing/x" "relative/../y"; do
    [[ $(portable_realpath -m -- "$p") == $(realpath -m -- "$p") ]] || fail "portable_realpath $p: $(portable_realpath -m -- "$p")"
  done
}

test_package_refuses_linux() {
  local msg
  if msg=$(package_main 2>&1); then fail "package ran on Linux"; fi
  [[ $msg == *"make package"* ]] || fail "Linux refusal message: $msg"
}

test_package_stops_at_the_first_failed_step() {
  # A fake macOS host whose release build fails: nothing after cargo build may run.
  local fake=$WORK/fail-bin root out
  root=$(cd "$(dirname "$RELEASE_SH")/.." && pwd)
  out=$root/target/release-test-$$
  mkdir -p "$fake"
  printf '#!/usr/bin/env bash\necho Darwin\n' >"$fake/uname"
  printf '#!/usr/bin/env bash\nif [[ $1 == pkgid ]]; then echo "x#apinata-cli@1.2.3"; exit 0; fi\nif [[ $1 == about ]]; then [[ $2 == --version ]] && echo cargo-about 0.9.3; exit 0; fi\necho "cargo $*" >>"%s/fail.log"\nexit 1\n' "$WORK" >"$fake/cargo"
  local t
  for t in codesign hdiutil gtar; do
    printf '#!/usr/bin/env bash\necho "%s $*" >>"%s/fail.log"\n' "$t" "$WORK" >"$fake/$t"
  done
  chmod +x "$fake"/*
  if PATH=$fake:$PATH OUT_DIR=$out bash "$RELEASE_SH" package >/dev/null 2>&1; then fail "package exited 0 after a failed build"; fi
  rm -rf -- "$out"
  [[ $(cat "$WORK/fail.log") == "cargo build --release --locked -p apinata-cli -p apinata-app" ]] \
    || fail "steps ran after the failed build: $(tr '\n' '|' <"$WORK/fail.log")"
}

test_macos_code_paths_avoid_bash4_only_syntax() {
  local bash4='mapfile|readarray|declare -A|local -A|\|&|;;&|\$\{[A-Za-z_]+(,,|\^\^|@[A-Za-z])'
  if grep -nE "$bash4" "$RELEASE_SH" | grep -vE '^[0-9]+:[[:space:]]*#'; then fail "release.sh uses bash 4 syntax"; fi
  if declare -f prepare_out_dir package_app cli_archive | grep -nE "$bash4"; then
    fail "a package-linux.sh function used on macOS uses bash 4 syntax"
  fi
}

test_macos_update_tarball_holds_only_the_bundle() {
  local out=$WORK/tarball
  mkdir -p "$out/Apinata.app/Contents/MacOS"
  printf exe >"$out/Apinata.app/Contents/MacOS/apinata-app"
  macos_update_tarball "$out" 1.2.3 aarch64 || fail "macos_update_tarball failed"
  local names
  names=$(tar -tzf "$out/Apinata_1.2.3_aarch64.app.tar.gz" | cut -d/ -f1 | sort -u)
  [[ $names == Apinata.app ]] || fail "top-level entries: $names"
  [[ -d $out/Apinata.app ]] || fail "the bundle was removed"
}

test_macos_dmg_also_writes_the_update_tarball() {
  local out=$WORK/mac2
  mkdir -p "$out/Apinata.app/Contents"
  printf '#!/usr/bin/env bash\n' >"$WORK/codesign2"
  printf '#!/usr/bin/env bash\ntouch "${@: -1}"\n' >"$WORK/hdiutil2"
  chmod +x "$WORK/codesign2" "$WORK/hdiutil2"
  CODESIGN=$WORK/codesign2 HDIUTIL=$WORK/hdiutil2 macos_dmg "$out" 1.2.3 x86_64 || fail "macos_dmg failed"
  [[ -f $out/Apinata_1.2.3_x86_64.app.tar.gz ]] || fail "no update tarball: $(ls "$out")"
}

test_update_map_lists_present_artifacts_by_target() {
  local d=$WORK/map
  mkdir -p "$d"
  touch "$d/apinata-app_1.2.3_x86_64.AppImage" "$d/Apinata_1.2.3_aarch64.app.tar.gz" \
    "$d/apinata-app_1.2.3_x64-setup-unsigned.exe" "$d/apinata-1.2.3-x86_64-unknown-linux-gnu" \
    "$d/apinata-1.2.3-x86_64-pc-windows-msvc.exe" "$d/apinata-1.2.3-x86_64-unknown-linux-gnu.tar.gz"
  local want="app-linux-x86_64 apinata-app_1.2.3_x86_64.AppImage
app-macos-aarch64 Apinata_1.2.3_aarch64.app.tar.gz
app-windows-x86_64 apinata-app_1.2.3_x64-setup-unsigned.exe
cli-linux-x86_64 apinata-1.2.3-x86_64-unknown-linux-gnu
cli-windows-x86_64 apinata-1.2.3-x86_64-pc-windows-msvc.exe"
  [[ $(update_map "$d" 1.2.3 | LC_ALL=C sort) == "$want" ]] || fail "map: $(update_map "$d" 1.2.3)"
}

test_update_manifest_strips_base_slashes_and_moves_the_manifest_out() {
  local d=$WORK/man out=$WORK/man-out
  mkdir -p "$d"
  touch "$d/apinata-1.2.3-x86_64-unknown-linux-gnu"
  printf '#!/usr/bin/env bash\necho "$*" >"%s/cargo.log"\nprintf "{}" >"${@: -1}/latest.json"\n' \
    "$WORK" >"$WORK/cargo-stub"
  chmod +x "$WORK/cargo-stub"
  CARGO=$WORK/cargo-stub update_manifest "$d" 1.2.3 https://h/dl// v1.2.3 "$out" || fail "update_manifest failed"
  [[ -f $out/latest.json && ! -e $d/latest.json ]] || fail "manifest not moved: $(ls "$d" "$out")"
  grep -q -- '^xtask update-manifest --version 1.2.3 --base-url https://h/dl/v1.2.3 --notes ' "$WORK/cargo.log" \
    || fail "cargo args: $(cat "$WORK/cargo.log")"
  CARGO=$WORK/cargo-stub update_manifest "$WORK/empty-$$" 1.2.3 https://h v1.2.3 "$out" 2>/dev/null \
    && fail "an empty directory must fail"
  return 0
}

test_stable_copies_names_each_platform_without_the_version() {
  local d=$WORK/st-dist out=$WORK/st-out
  mkdir -p "$d"
  local f
  for f in apinata-app_1.2.3_x86_64.AppImage apinata-app_1.2.3_aarch64.AppImage \
    apinata-app_1.2.3_amd64.deb apinata-app_1.2.3_arm64.deb \
    Apinata_1.2.3_aarch64.dmg Apinata_1.2.3_x86_64.dmg Apinata_1.2.3_x86_64.app.tar.gz \
    apinata-app_1.2.3_x64-setup-unsigned.exe apinata-1.2.3-x86_64-pc-windows-msvc.exe \
    apinata-1.2.3-x86_64-unknown-linux-gnu.tar.gz SHA256SUMS; do
    printf '%s' "$f" >"$d/$f"
  done
  stable_copies "$d" "$out" || { fail "stable_copies failed"; return 0; }
  local want="apinata-linux-aarch64.AppImage
apinata-linux-amd64.deb
apinata-linux-arm64.deb
apinata-linux-x86_64.AppImage
apinata-macos-aarch64.dmg
apinata-macos-x86_64.dmg
apinata-windows-setup.exe"
  [[ $(ls "$out" | LC_ALL=C sort) == "$want" ]] || fail "names: $(ls "$out")"
  cmp -s "$out/apinata-windows-setup.exe" "$d/apinata-app_1.2.3_x64-setup-unsigned.exe" || fail "bytes differ"
  [[ $(ls "$d" | wc -l) -eq 11 ]] || fail "dist was changed"
  # A signed installer name works the same, and an absent platform is skipped.
  rm -rf "$out" "$d"; mkdir -p "$d"
  touch "$d/apinata-app_1.2.3_x64-setup.exe"
  stable_copies "$d" "$out" || fail "windows-only failed"
  [[ $(ls "$out") == apinata-windows-setup.exe ]] || fail "windows-only: $(ls "$out")"
}

test_stable_copies_refuses_ambiguous_or_empty_input() {
  local d=$WORK/st2-dist
  mkdir -p "$d"
  touch "$d/apinata-app_1.2.3_amd64.deb" "$d/apinata-app_1.2.4_amd64.deb"
  if stable_copies "$d" "$WORK/st2-out" 2>/dev/null; then fail "two matches accepted"; fi
  rm -rf "$d"; mkdir -p "$d"; touch "$d/SHA256SUMS"
  if stable_copies "$d" "$WORK/st2-out2" 2>/dev/null; then fail "no installers accepted"; fi
  return 0
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "release tests passed"
