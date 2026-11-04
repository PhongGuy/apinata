#!/usr/bin/env bash
# Tests for scripts/install.sh: every OS branch runs on Linux against a stub cargo, and nothing
# is written outside this suite's own temporary directory.
set -euo pipefail
SCRIPT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/install.sh
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

# The real xtask keeps the caller's toolchain and target dir; the faked HOME must not hide them.
export REAL_CARGO XTASK_TARGET_DIR=${CARGO_TARGET_DIR:-}
REAL_CARGO=$(command -v cargo)
export CARGO_HOME=${CARGO_HOME:-$HOME/.cargo} RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
export HOME=$WORK/home CARGO_TARGET_DIR=$WORK/target INSTALL_CARGO=$WORK/bin/cargo
export PATH=$WORK/bin:$PATH
unset INSTALL_STAGE_DIR PREFIX DESTDIR APPLICATIONS_DIR LOCALAPPDATA INSTALL_OS
mkdir -p "$HOME" "$WORK/bin"

cat >"$WORK/bin/cargo" <<'EOF'
#!/usr/bin/env bash
# Stub cargo: `build` fakes release binaries, `packager` fakes Apinata.app, `xtask` runs the real one.
set -euo pipefail
case $1 in
  build) mkdir -p "$CARGO_TARGET_DIR/release"
         for b in apinata apinata-app apinata.exe apinata-app.exe; do printf 'bin %s\n' "$b" >"$CARGO_TARGET_DIR/release/$b"; done ;;
  xtask) if [[ $2 == packager-config ]]; then exit 0; fi; shift
         if [[ -n $XTASK_TARGET_DIR ]]; then exec env CARGO_TARGET_DIR="$XTASK_TARGET_DIR" "$REAL_CARGO" xtask "$@"; fi
         exec env -u CARGO_TARGET_DIR "$REAL_CARGO" xtask "$@" ;;
  packager) mkdir -p "$INSTALL_STAGE_DIR/Apinata.app/Contents/MacOS"; touch "$INSTALL_STAGE_DIR/Apinata.app/Contents/MacOS/apinata-app" ;;
esac
EOF
# The cache refreshers only log, so a test can see they ran and on which directory.
for tool in update-desktop-database gtk-update-icon-cache; do
  printf '#!/usr/bin/env bash\necho "%s $*" >>"%s/refresh.log"\n' "$tool" "$WORK" >"$WORK/bin/$tool"
done
chmod +x "$WORK"/bin/*

test_linux_install_then_uninstall_removes_exactly_what_it_wrote() {
  local p=$WORK/l/prefix; mkdir -p "$p/bin"; echo keep >"$p/bin/other-tool"
  INSTALL_OS=Linux PREFIX=$p bash "$SCRIPT" install >/dev/null || fail "linux install failed"
  for f in bin/apinata bin/apinata-app share/applications/apinata-app.desktop share/icons/hicolor/256x256/apps/apinata-app.png; do
    [[ -f $p/$f ]] || fail "missing $f"
  done
  [[ -x $p/bin/apinata-app ]] || fail "apinata-app not executable"
  grep -qx "Name=Apinata" "$p/share/applications/apinata-app.desktop" || fail "desktop name"
  grep -qx "Exec=$p/bin/apinata-app" "$p/share/applications/apinata-app.desktop" || fail "desktop exec"
  grep -qx "StartupWMClass=apinata-app" "$p/share/applications/apinata-app.desktop" || fail "wm class"
  grep -qx "update-desktop-database $p/share/applications" "$WORK/refresh.log" || fail "desktop database not refreshed"
  INSTALL_OS=Linux PREFIX=$p bash "$SCRIPT" uninstall >/dev/null || fail "linux uninstall failed"
  local left; left=$(cd "$p" && find . -type f | sort)
  [[ $left == "./bin/other-tool" ]] || fail "left behind: $left"
}

test_destdir_moves_files_but_not_exec() {
  local d=$WORK/d/stage
  INSTALL_OS=Linux PREFIX=/usr/local DESTDIR=$d bash "$SCRIPT" install >/dev/null || fail "destdir install failed"
  [[ -f $d/usr/local/bin/apinata ]] || fail "binary not under DESTDIR"
  grep -qx "Exec=/usr/local/bin/apinata-app" "$d/usr/local/share/applications/apinata-app.desktop" || fail "exec includes DESTDIR"
  ! grep -q "$d" "$WORK/refresh.log" 2>/dev/null || fail "caches refreshed for a staged install"
}

test_macos_branch_installs_the_app_bundle_and_cli() {
  local p=$WORK/m/prefix apps=$WORK/m/Applications; mkdir -p "$apps/Apinata.app/stale"
  INSTALL_OS=Darwin PREFIX=$p APPLICATIONS_DIR=$apps INSTALL_STAGE_DIR=$WORK/m/stage bash "$SCRIPT" install >/dev/null || fail "mac install failed"
  [[ -f $apps/Apinata.app/Contents/MacOS/apinata-app ]] || fail "bundle not copied"
  [[ ! -e $apps/Apinata.app/stale ]] || fail "old bundle kept"
  [[ -f $p/bin/apinata ]] || fail "cli not installed"
  INSTALL_OS=Darwin PREFIX=$p APPLICATIONS_DIR=$apps bash "$SCRIPT" uninstall >/dev/null || fail "mac uninstall failed"
  [[ ! -e $apps/Apinata.app && ! -e $p/bin/apinata ]] || fail "mac uninstall left files"
}

test_windows_branch_copies_exes_to_local_programs() {
  local l=$WORK/w/local
  INSTALL_OS=MINGW64_NT-10.0 LOCALAPPDATA=$l bash "$SCRIPT" install >/dev/null || fail "win install failed"
  [[ -f $l/Programs/Apinata/apinata.exe && -f $l/Programs/Apinata/apinata-app.exe ]] || fail "exes missing"
  INSTALL_OS=MINGW64_NT-10.0 LOCALAPPDATA=$l bash "$SCRIPT" uninstall >/dev/null || fail "win uninstall failed"
  [[ ! -e $l/Programs/Apinata ]] || fail "win dir left"
}

test_a_relative_prefix_is_refused() {
  local err
  if err=$(cd "$WORK" && INSTALL_OS=Linux PREFIX=rel bash "$SCRIPT" install 2>&1); then fail "relative PREFIX accepted"; fi
  [[ $err == *"PREFIX must be absolute"* ]] || fail "message: $err"
  [[ ! -e $WORK/rel ]] || fail "relative PREFIX wrote files"
}

test_unknown_os_is_refused() {
  local err
  if err=$(INSTALL_OS=Plan9 PREFIX=$WORK/u bash "$SCRIPT" install 2>&1); then fail "unknown OS accepted"; fi
  [[ $err == *"unsupported OS Plan9"* ]] || fail "message: $err"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "install tests passed"
