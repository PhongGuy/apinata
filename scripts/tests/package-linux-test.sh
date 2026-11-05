#!/usr/bin/env bash
# Tests for the functions in scripts/package-linux.sh.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../package-linux.sh"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

test_seed_tools_installs_verified_files_and_restores_patched_ones() {
  export XDG_CACHE_HOME=$WORK/cache-ok
  printf 'tool bytes\n' >"$WORK/tool"
  local sum; sum=$(sha256sum "$WORK/tool" | cut -d' ' -f1)
  cat >"$WORK/manifest" <<EOF
# comment line
x86_64 linuxdeploy-x86_64.AppImage $sum file://$WORK/tool
aarch64 other-arch.AppImage $sum file://$WORK/missing
EOF
  seed_tools "$WORK/manifest" x86_64 || fail "seed_tools returned non-zero"
  local dest=$XDG_CACHE_HOME/.cargo-packager/AppImage/linuxdeploy-x86_64.AppImage
  [[ -f $dest ]] || fail "tool not installed"
  [[ $(stat -c %a "$dest") == 764 ]] || fail "mode is $(stat -c %a "$dest"), want 764"
  [[ ! -e $XDG_CACHE_HOME/.cargo-packager/AppImage/other-arch.AppImage ]] || fail "other arch installed"
  printf 'XXX' | dd of="$dest" bs=1 seek=8 conv=notrunc status=none
  rm "$WORK/tool"
  seed_tools "$WORK/manifest" x86_64 || fail "second seed_tools returned non-zero"
  cmp -s "$dest" <(printf 'tool bytes\n') || fail "patched tool not restored from the cache"
}

test_seed_tools_rejects_a_hash_mismatch() {
  export XDG_CACHE_HOME=$WORK/cache-bad
  printf 'tampered\n' >"$WORK/bad"
  echo "x86_64 linuxdeploy-x86_64.AppImage $(printf '0%.0s' {1..64}) file://$WORK/bad" >"$WORK/manifest-bad"
  if seed_tools "$WORK/manifest-bad" x86_64 2>/dev/null; then fail "mismatch accepted"; fi
  [[ ! -e $XDG_CACHE_HOME/.cargo-packager/AppImage/linuxdeploy-x86_64.AppImage ]] || fail "bad tool installed"
  [[ -z $(find "$XDG_CACHE_HOME/apinata" -type f) ]] || fail "bad download left in the cache"
}

test_cli_archive_is_reproducible_and_complete() {
  printf '#!/bin/sh\necho apinata 9.9.9\n' >"$WORK/apinata-bin"
  printf 'mit\n' >"$WORK/LICENSE-MIT"
  printf 'apache\n' >"$WORK/LICENSE-APACHE"
  printf 'readme\n' >"$WORK/README.md"
  mkdir -p "$WORK/licenses"
  printf 'chai\n' >"$WORK/licenses/chai-MIT.txt"
  local files=("$WORK/LICENSE-MIT" "$WORK/LICENSE-APACHE" "$WORK/README.md" "$WORK/licenses")
  cli_archive "$WORK/apinata-bin" 9.9.9 x86_64-unknown-linux-gnu "$WORK/out1" "${files[@]}"
  touch "$WORK/README.md"
  sleep 1
  cli_archive "$WORK/apinata-bin" 9.9.9 x86_64-unknown-linux-gnu "$WORK/out2" "${files[@]}"
  local a=$WORK/out1/apinata-9.9.9-x86_64-unknown-linux-gnu.tar.gz
  cmp -s "$a" "$WORK/out2/apinata-9.9.9-x86_64-unknown-linux-gnu.tar.gz" || fail "archive not reproducible"
  local listing; listing=$(tar -tzf "$a" | LC_ALL=C sort | tr '\n' ' ')
  local want="apinata-9.9.9-x86_64-unknown-linux-gnu/ apinata-9.9.9-x86_64-unknown-linux-gnu/LICENSE-APACHE apinata-9.9.9-x86_64-unknown-linux-gnu/LICENSE-MIT apinata-9.9.9-x86_64-unknown-linux-gnu/README.md apinata-9.9.9-x86_64-unknown-linux-gnu/apinata apinata-9.9.9-x86_64-unknown-linux-gnu/licenses/ apinata-9.9.9-x86_64-unknown-linux-gnu/licenses/chai-MIT.txt "
  [[ $listing == "$want" ]] || fail "archive listing: $listing"
  tar -tvzf "$a" | grep -q '^-rwxr-xr-x 0/0 .*/apinata$' || fail "binary not executable or not owned by 0/0"
}

test_write_sums_covers_artifacts_by_bare_name() {
  local d=$WORK/sums
  mkdir -p "$d"
  printf a >"$d/apinata-1-x.tar.gz"; printf b >"$d/apinata_1_amd64.deb"
  printf c >"$d/apinata-app_1_x86_64.AppImage"; printf d >"$d/notes.txt"
  write_sums "$d"
  [[ $(wc -l <"$d/SHA256SUMS") -eq 3 ]] || fail "SHA256SUMS has $(wc -l <"$d/SHA256SUMS") lines, want 3"
  grep -q '/' "$d/SHA256SUMS" && fail "SHA256SUMS contains a path"
  (cd "$d" && sha256sum -c --quiet SHA256SUMS) || fail "SHA256SUMS does not verify"
}

test_missing_depends_reports_only_undeclared_libraries() {
  local depends="libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1, libwayland-client0, libwayland-egl1, libegl1"
  local all="libc.so.6 libm.so.6 libgcc_s.so.1 ld-linux-x86-64.so.2 libxcb.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0 libvulkan.so.1 libwayland-client.so.0 libwayland-egl.so.1 libEGL.so.1"
  [[ -z $(missing_depends "$all" "$depends") ]] || fail "declared set reported: $(missing_depends "$all" "$depends")"
  [[ -z $(missing_depends "ld-linux-aarch64.so.1" "$depends") ]] || fail "aarch64 loader reported: $(missing_depends "ld-linux-aarch64.so.1" "$depends")"
  [[ $(missing_depends "libEGL.so.1" "libxcb1, libvulkan1") == "libEGL.so.1 (libegl1)" ]] || fail "undeclared libegl1 not reported"
  [[ $(missing_depends "libfoo.so.3" "$depends") == "libfoo.so.3 (unknown)" ]] || fail "unknown soname not reported"
  [[ $(missing_depends "libxcb.so.1" "libxcb1-dev") == "libxcb.so.1 (libxcb1)" ]] || fail "prefix match accepted"
}

test_seed_tools_reads_a_final_line_without_a_newline() {
  export XDG_CACHE_HOME=$WORK/cache-nonl
  printf 'last tool\n' >"$WORK/tool-nonl"
  local sum; sum=$(sha256sum "$WORK/tool-nonl" | cut -d' ' -f1)
  printf 'x86_64 AppRun-x86_64 %s file://%s' "$sum" "$WORK/tool-nonl" >"$WORK/manifest-nonl"
  seed_tools "$WORK/manifest-nonl" x86_64 || fail "seed_tools returned non-zero"
  [[ -f $XDG_CACHE_HOME/.cargo-packager/AppImage/AppRun-x86_64 ]] || fail "final line without a newline skipped"
}

test_prepare_out_dir_refuses_unsafe_directories() {
  local root=$WORK/repo
  mkdir -p "$root/home" "$WORK/outside"
  local bad
  for bad in "" / "$root" "$root/" "$root/home" "$WORK/outside" "$root/../outside" "$root/x/../.."; do
    if HOME=$root/home prepare_out_dir "$root" "$bad" 2>/dev/null; then fail "accepted '$bad'"; fi
  done
  [[ -d $WORK/outside && -d $root/home ]] || fail "a refused directory was touched"
  local msg; msg=$(HOME=$root/home prepare_out_dir "$root" "$root/x/../.." 2>&1) || true
  [[ $msg == *"$WORK"* && $msg != *x/../..* && $msg == *OUT_DIR* ]] || fail "refusal does not name the resolved path and OUT_DIR: $msg"
}

test_prepare_out_dir_removes_only_known_artifacts() {
  local root=$WORK/repo2 out=$WORK/repo2/target/dist/linux
  mkdir -p "$out/.cargo-packager/x"
  touch "$out/apinata-1-x.tar.gz" "$out/apinata-app_1_amd64.deb" "$out/apinata-app_1_x86_64.AppImage" \
    "$out/SHA256SUMS" "$out/keep.txt"
  HOME=$WORK prepare_out_dir "$root" "$out" || fail "valid directory refused"
  [[ $(ls -A "$out") == keep.txt ]] || fail "left: $(ls -A "$out" | tr '\n' ' ')"
  HOME=$WORK prepare_out_dir "$root" "$root/target/dist/new" || fail "new directory refused"
  [[ -d $root/target/dist/new ]] || fail "new directory not created"
}

# A stand-in cargo: `xtask packager-config` writes --output; `packager --config F` records
# F's mode and fails when FAKE_PACKAGER_FAIL is set.
write_fake_cargo() {
  cat >"$WORK/fake-cargo" <<'EOF'
#!/usr/bin/env bash
case $1 in
  xtask) while [[ $# -gt 0 ]]; do [[ $1 == --output ]] && { printf '{"secret":1}' >"$2"; }; shift; done ;;
  packager)
    cfg=$3
    echo "$cfg" >"$FAKE_LOG"
    stat -c %a "$cfg" >>"$FAKE_LOG"
    stat -c %a "$(dirname "$cfg")" >>"$FAKE_LOG"
    umask >>"$FAKE_LOG"
    echo "$5" >>"$FAKE_LOG"
    [[ -z ${FAKE_PACKAGER_FAIL:-} ]] ;;
esac
EOF
  chmod +x "$WORK/fake-cargo"
}

test_package_app_removes_the_config_on_success_and_failure() {
  write_fake_cargo
  export PACKAGE_LINUX_CARGO=$WORK/fake-cargo FAKE_LOG=$WORK/fake.log
  package_app "$WORK/o" || fail "package_app failed with a passing packager"
  local cfg; cfg=$(sed -n 1p "$FAKE_LOG")
  [[ $(sed -n 2p "$FAKE_LOG") == 600 ]] || fail "config mode $(sed -n 2p "$FAKE_LOG"), want 600"
  [[ $(sed -n 3p "$FAKE_LOG") == 700 ]] || fail "config dir mode $(sed -n 3p "$FAKE_LOG"), want 700"
  [[ $(sed -n 4p "$FAKE_LOG") == "$(umask)" ]] || fail "packager ran with umask $(sed -n 4p "$FAKE_LOG")"
  [[ ! -e $cfg && ! -e $(dirname "$cfg") ]] || fail "config left after success"
  [[ $cfg != "$PWD"/* ]] || fail "config written under the working directory"

  if FAKE_PACKAGER_FAIL=1 package_app "$WORK/o"; then fail "packager failure not reported"; fi
  cfg=$(sed -n 1p "$FAKE_LOG")
  [[ ! -e $cfg && ! -e $(dirname "$cfg") ]] || fail "config left after failure"
  unset PACKAGE_LINUX_CARGO FAKE_LOG
}

test_free_display_skips_taken_numbers() {
  mkdir -p "$WORK/x11" "$WORK/locks"
  touch "$WORK/x11/X90" "$WORK/locks/.X91-lock"
  [[ $(free_display "$WORK/x11" "$WORK/locks") == 92 ]] || fail "free_display gave $(free_display "$WORK/x11" "$WORK/locks")"
}

test_ld_failures_finds_missing_libraries() {
  printf '     12:\tfind library=libvulkan.so.1 [0]; searching\n' >"$WORK/ok.log"
  [[ -z $(ld_failures "$WORK/ok.log") ]] || fail "clean log reported"
  printf 'apinata-app: error while loading shared libraries: libxkbcommon-x11.so.0: cannot open shared object file\n' >"$WORK/bad.log"
  [[ -n $(ld_failures "$WORK/bad.log") ]] || fail "missing library not reported"
}

test_deb_arch_maps_machines_to_debian_names() {
  [[ $(deb_arch x86_64) == amd64 ]] || fail "x86_64 gave $(deb_arch x86_64)"
  [[ $(deb_arch aarch64) == arm64 ]] || fail "aarch64 gave $(deb_arch aarch64)"
  if deb_arch riscv64 2>/dev/null; then fail "riscv64 accepted"; fi
}

test_write_sums_covers_macos_and_windows_artifacts() {
  local d=$WORK/sums-all
  mkdir -p "$d"
  printf a >"$d/Apinata_1_aarch64.dmg"; printf b >"$d/apinata-app_1_x64-setup.exe"
  printf c >"$d/apinata-1-x86_64-pc-windows-msvc.zip"; printf d >"$d/apinata-1-x.tar.gz"
  printf e >"$d/notes.txt"
  write_sums "$d"
  [[ $(cut -d' ' -f3 "$d/SHA256SUMS" | tr '\n' ' ') == "Apinata_1_aarch64.dmg apinata-1-x.tar.gz apinata-1-x86_64-pc-windows-msvc.zip apinata-app_1_x64-setup.exe " ]] \
    || fail "SHA256SUMS names: $(cut -d' ' -f3 "$d/SHA256SUMS" | tr '\n' ' ')"
}

test_prepare_out_dir_removes_macos_and_windows_artifacts() {
  local root=$WORK/repo3 out=$WORK/repo3/target/dist/macos
  mkdir -p "$out/Apinata.app/Contents"
  touch "$out/Apinata_1_aarch64.dmg" "$out/apinata-app_1_x64-setup-unsigned.exe" "$out/apinata-app_1_x64-setup.exe" \
    "$out/apinata-1-x86_64-pc-windows-msvc.zip" "$out/keep.txt"
  HOME=$WORK prepare_out_dir "$root" "$out" || fail "valid directory refused"
  [[ $(ls -A "$out") == keep.txt ]] || fail "left: $(ls -A "$out" | tr '\n' ' ')"
}

test_package_app_passes_the_requested_formats() {
  write_fake_cargo
  export PACKAGE_LINUX_CARGO=$WORK/fake-cargo FAKE_LOG=$WORK/fake-formats.log
  package_app "$WORK/o" || fail "package_app failed"
  [[ $(sed -n 5p "$FAKE_LOG") == deb,appimage ]] || fail "default formats: $(sed -n 5p "$FAKE_LOG")"
  package_app "$WORK/o" dmg || fail "package_app dmg failed"
  [[ $(sed -n 5p "$FAKE_LOG") == dmg ]] || fail "dmg formats: $(sed -n 5p "$FAKE_LOG")"
  unset PACKAGE_LINUX_CARGO FAKE_LOG
}

test_cli_archive_uses_the_tar_override() {
  printf '#!/usr/bin/env bash\ntouch "%s/tar-used"\nexec tar "$@"\n' "$WORK" >"$WORK/fake-tar"
  chmod +x "$WORK/fake-tar"
  printf 'bin\n' >"$WORK/bin-t"
  TAR=$WORK/fake-tar cli_archive "$WORK/bin-t" 1.0.0 aarch64-apple-darwin "$WORK/tout"
  [[ -f $WORK/tar-used ]] || fail "TAR override ignored"
  [[ -f $WORK/tout/apinata-1.0.0-aarch64-apple-darwin.tar.gz ]] || fail "archive not written"
}

test_desktop_entry_ok_needs_the_class_and_icon() {
  printf '[Desktop Entry]\nIcon=apinata-app\nStartupWMClass=apinata-app\n' >"$WORK/good.desktop"
  printf '[Desktop Entry]\nIcon=apinata-app\n' >"$WORK/noclass.desktop"
  printf '[Desktop Entry]\nIcon=other\nStartupWMClass=apinata-app\n' >"$WORK/badicon.desktop"
  desktop_entry_ok "$WORK/good.desktop" apinata-app || fail "good entry rejected"
  local err
  if err=$(desktop_entry_ok "$WORK/noclass.desktop" apinata-app 2>&1); then fail "entry without class passed"; fi
  [[ $err == *"lacks StartupWMClass=apinata-app"* ]] || fail "error was: $err"
  if desktop_entry_ok "$WORK/badicon.desktop" apinata-app 2>/dev/null; then fail "wrong icon passed"; fi
  if desktop_entry_ok "$WORK/missing.desktop" apinata-app 2>/dev/null; then fail "missing file passed"; fi
}

test_cli_raw_installs_an_executable_copy() {
  local d=$WORK/raw
  mkdir -p "$d"
  printf bin >"$WORK/raw-bin"
  cli_raw "$WORK/raw-bin" 1.2.3 x86_64-pc-windows-msvc "$d" .exe || fail "cli_raw failed"
  local f=$d/apinata-1.2.3-x86_64-pc-windows-msvc.exe
  [[ -x $f && $(cat "$f") == bin ]] || fail "raw binary missing or not executable: $(ls -l "$d")"
}

test_sums_and_cleanup_cover_raw_binaries_and_update_tarballs() {
  local root=$WORK/repo4 out=$WORK/repo4/target/dist/macos
  mkdir -p "$out"
  printf a >"$out/apinata-1-aarch64-apple-darwin"; printf b >"$out/Apinata_1_aarch64.app.tar.gz"
  write_sums "$out"
  [[ $(cut -d' ' -f3 "$out/SHA256SUMS" | tr '\n' ' ') == "Apinata_1_aarch64.app.tar.gz apinata-1-aarch64-apple-darwin " ]] \
    || fail "SHA256SUMS names: $(cut -d' ' -f3 "$out/SHA256SUMS" | tr '\n' ' ')"
  touch "$out/keep.txt"
  HOME=$WORK prepare_out_dir "$root" "$out" || fail "valid directory refused"
  [[ $(ls -A "$out") == keep.txt ]] || fail "left: $(ls -A "$out" | tr '\n' ' ')"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "package-linux tests passed"
