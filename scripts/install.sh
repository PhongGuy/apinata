#!/usr/bin/env bash
# Installs or uninstalls the app and the CLI for the current user:
#   bash scripts/install.sh install|uninstall
# Env: PREFIX (default ~/.local), DESTDIR, APPLICATIONS_DIR (macOS), LOCALAPPDATA (Windows).
# INSTALL_OS and INSTALL_CARGO override `uname -s` and `cargo` so tests can drive every branch.
# Must stay bash 3.2 safe, since macOS ships /bin/bash 3.2.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ACTION=${1:-}
OS=${INSTALL_OS:-$(uname -s)}
CARGO_BIN=${INSTALL_CARGO:-cargo}
PREFIX=${PREFIX:-$HOME/.local}
DESTDIR=${DESTDIR:-}
D=$DESTDIR$PREFIX
RELEASE=${CARGO_TARGET_DIR:-$ROOT/target}/release

die() { echo "install: $*" >&2; exit 1; }

# A relative PREFIX would land files beside whatever directory each step runs in.
[[ $PREFIX == /* ]] || die "PREFIX must be absolute, got $PREFIX"

build() {
  (cd "$ROOT" && "$CARGO_BIN" build --release --locked -p apinata-cli -p apinata-app)
}

# Copy FILE to DEST with mode 755, creating DEST's directory; BSD install has no -D.
put() {
  mkdir -p "$(dirname "$2")"
  install -m 755 "$1" "$2"
  echo "wrote $2"
}

# Remove one named file, saying so only when it was there.
drop() {
  if [[ -e $1 ]]; then rm -f "$1"; echo "removed $1"; fi
}

# The desktop entry's Exec names the final PREFIX path, so a DESTDIR stage installs correctly.
desktop() {
  (cd "$ROOT" && "$CARGO_BIN" xtask desktop --data-home "$D/share" --binary "$PREFIX/bin/apinata-app" "$@")
}

linux_install() {
  build
  put "$RELEASE/apinata" "$D/bin/apinata"
  put "$RELEASE/apinata-app" "$D/bin/apinata-app"
  desktop
  # Refresh the launcher caches only for a live install; a staged one belongs to a packager.
  if [[ -z $DESTDIR ]]; then
    if command -v update-desktop-database >/dev/null 2>&1; then
      update-desktop-database "$D/share/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
      gtk-update-icon-cache -q -t "$D/share/icons/hicolor" || true
    fi
  fi
}

linux_uninstall() {
  drop "$D/bin/apinata"
  drop "$D/bin/apinata-app"
  desktop --remove
}

# Build the bundle with the release packager into a stage dir, then replace the installed one.
macos_install() {
  build
  local stage=${INSTALL_STAGE_DIR:-} own=0 apps=${APPLICATIONS_DIR:-$HOME/Applications}
  if [[ -z $stage ]]; then stage=$(mktemp -d); own=1; fi
  PACKAGE_LINUX_CARGO=$CARGO_BIN
  # shellcheck source=package-linux.sh
  source "$ROOT/scripts/package-linux.sh"
  # The installed bundle is only replaced once a new one exists; a failed build leaves no stage behind.
  if ! (cd "$ROOT" && package_app "$stage" app) || [[ ! -d $stage/Apinata.app ]]; then
    if ((own)); then rm -rf "$stage"; fi
    die "packaging did not produce $stage/Apinata.app"
  fi
  mkdir -p "$apps"
  rm -rf "$apps/Apinata.app"
  cp -R "$stage/Apinata.app" "$apps/Apinata.app"
  echo "wrote $apps/Apinata.app"
  if ((own)); then rm -rf "$stage"; fi
  put "$RELEASE/apinata" "$D/bin/apinata"
}

macos_uninstall() {
  local apps=${APPLICATIONS_DIR:-$HOME/Applications}
  if [[ -e $apps/Apinata.app ]]; then rm -rf "$apps/Apinata.app"; echo "removed $apps/Apinata.app"; fi
  drop "$D/bin/apinata"
}

windows_dir() {
  [[ -n ${LOCALAPPDATA:-} ]] || die "LOCALAPPDATA is not set"
  local base=$LOCALAPPDATA
  if command -v cygpath >/dev/null 2>&1; then base=$(cygpath -u "$LOCALAPPDATA"); fi
  echo "$base/Programs/Apinata"
}

windows_install() {
  local dir
  dir=$(windows_dir)
  build
  put "$RELEASE/apinata.exe" "$dir/apinata.exe"
  put "$RELEASE/apinata-app.exe" "$dir/apinata-app.exe"
  echo "note: add $dir to your PATH to run apinata from a terminal"
}

windows_uninstall() {
  local dir
  dir=$(windows_dir)
  drop "$dir/apinata.exe"
  drop "$dir/apinata-app.exe"
  if [[ -d $dir ]] && rmdir "$dir" 2>/dev/null; then echo "removed $dir"; fi
}

case $ACTION in
  install | uninstall) ;;
  *) die "usage: install.sh install|uninstall" ;;
esac
case $OS in
  Linux) platform=linux ;;
  Darwin) platform=macos ;;
  MINGW* | MSYS* | CYGWIN*) platform=windows ;;
  *) die "unsupported OS $OS" ;;
esac
"${platform}_$ACTION"
