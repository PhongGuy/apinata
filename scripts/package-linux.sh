#!/usr/bin/env bash
# Builds the Linux release artifacts: a CLI tarball, a .deb, an AppImage and
# SHA256SUMS; release.sh reuses its helpers for macOS and Windows. Sourcing
# this file only defines functions; running it calls main.
set -euo pipefail

# The cache that holds verified tool downloads, keyed by their sha256.
tools_cache_dir() {
  echo "${XDG_CACHE_HOME:-$HOME/.cache}/apinata/appimage-tools"
}

# Seed cargo-packager's AppImage tools dir with the pinned tools for ARCH.
# Each tool is downloaded once into a hash-keyed cache, then reinstalled on
# every run, because cargo-packager patches linuxdeploy in place.
seed_tools() {
  local manifest=$1 arch=$2
  local cache dest_dir
  cache=$(tools_cache_dir)
  dest_dir="${XDG_CACHE_HOME:-$HOME/.cache}/.cargo-packager/AppImage"
  mkdir -p "$cache" "$dest_dir"
  local line_arch name sum url
  # The `|| -n` clause keeps a final line that lacks a trailing newline.
  while read -r line_arch name sum url || [[ -n ${line_arch:-} ]]; do
    [[ -z ${line_arch:-} || $line_arch == \#* || $line_arch != "$arch" ]] && continue
    local cached=$cache/$sum
    if [[ ! -f $cached ]]; then
      curl -sSLf -A 'apinata-package-linux' -o "$cached.part" "$url" || {
        rm -f "$cached.part"
        echo "download failed: $url" >&2
        return 1
      }
      local got
      got=$(sha256sum "$cached.part" | cut -d' ' -f1)
      if [[ $got != "$sum" ]]; then
        rm -f "$cached.part"
        echo "sha256 mismatch for $name: got $got, want $sum" >&2
        return 1
      fi
      mv "$cached.part" "$cached"
    fi
    install -m 0764 "$cached" "$dest_dir/$name"
  done <"$manifest"
}

# Copy each FILE into DEST by name; a directory is copied as DEST/<name>/ holding its *.txt files.
stage_files() {
  local dest=$1 f t
  shift
  for f in "$@"; do
    if [[ -d $f ]]; then
      install -d -m 0755 "$dest/$(basename "$f")"
      for t in "$f"/*.txt; do install -m 0644 "$t" "$dest/$(basename "$f")/$(basename "$t")"; done
    else
      install -m 0644 "$f" "$dest/$(basename "$f")"
    fi
  done
}

# Write OUT/apinata-VERSION-TARGET.tar.gz holding the binary and FILEs.
# Staging with install and fixing tar's order, owner and mtime makes the
# archive byte-reproducible for a given SOURCE_DATE_EPOCH.
cli_archive() {
  local binary=$1 version=$2 target=$3 out=$4
  shift 4
  local name=apinata-$version-$target
  local stage
  stage=$(mktemp -d)
  install -d -m 0755 "$stage/$name"
  install -m 0755 "$binary" "$stage/$name/apinata"
  local f
  stage_files "$stage/$name" "$@"
  mkdir -p "$out"
  "${TAR:-tar}" -C "$stage" --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@${SOURCE_DATE_EPOCH:-0}" -cf - "$name" | gzip -n -9 >"$out/$name.tar.gz"
  rm -rf "$stage"
}

# Install BINARY as OUT/apinata-VERSION-TARGET[EXT] (mode 0755): the raw CLI the updater fetches.
cli_raw() {
  local binary=$1 version=$2 target=$3 out=$4 ext=${5:-}
  mkdir -p "$out"
  install -m 0755 "$binary" "$out/apinata-$version-$target$ext"
}

# Write DIR/SHA256SUMS over the release artifacts anywhere under DIR, each named by
# its path relative to DIR in byte order. Fails, leaving no SHA256SUMS, when none is found.
write_sums() {
  local dir=$1
  (
    cd "$dir" || exit 1
    find . -type f \( -name '*.tar.gz' -o -name '*.deb' -o -name '*.AppImage' \
      -o -name '*.dmg' -o -name '*.exe' -o -name '*.zip' -o -name 'apinata-[0-9]*' \) \
      -printf '%P\n' | LC_ALL=C sort | xargs -r -d '\n' sha256sum >SHA256SUMS || exit 1
    if [[ ! -s SHA256SUMS ]]; then
      rm -f SHA256SUMS
      echo "release: no release artifacts under $dir to checksum" >&2
      exit 1
    fi
  )
}

# Map a soname to the Debian package that provides it: one of the declared
# Depends, "essential" for the C runtime (aarch64 loader is .so.1), or "unknown".
soname_package() {
  case $1 in
    libxcb.so.1) echo libxcb1 ;;
    libxkbcommon.so.0) echo libxkbcommon0 ;;
    libxkbcommon-x11.so.0) echo libxkbcommon-x11-0 ;;
    libvulkan.so.1) echo libvulkan1 ;;
    libwayland-client.so.0) echo libwayland-client0 ;;
    libwayland-egl.so.1) echo libwayland-egl1 ;;
    libEGL.so.1) echo libegl1 ;;
    libc.so.6 | libm.so.6 | libgcc_s.so.1 | ld-linux-*.so.[12]) echo essential ;;
    *) echo unknown ;;
  esac
}

# Print "SONAME (PACKAGE)" for each soname in the space-separated SONAMES
# whose package is unknown or not an exact entry of the comma-separated DEPENDS.
missing_depends() {
  local sonames=$1 depends=$2
  local -A declared=()
  local entry entries
  IFS=',' read -ra entries <<<"$depends"
  for entry in "${entries[@]}"; do
    entry=${entry//[[:space:]]/}
    [[ -n $entry ]] && declared[$entry]=1
  done
  local so pkg
  for so in $sonames; do
    pkg=$(soname_package "$so")
    [[ $pkg == essential ]] && continue
    if [[ $pkg == unknown || -z ${declared[$pkg]:-} ]]; then
      echo "$so ($pkg)"
    fi
  done
}

# The sorted sonames an ELF loads: its NEEDED entries plus the ones it names
# as strings for dlopen, as wgpu and winit do.
binary_sonames() {
  local elf=$1
  {
    readelf -d "$elf" | sed -n 's/.*Shared library: \[\(.*\)\].*/\1/p'
    strings "$elf" | grep -oE 'lib[A-Za-z0-9_+-]+\.so\.[0-9]+' || true
  } | LC_ALL=C sort -u
}

# Validate OUT and empty it of earlier artifacts only. OUT must be a dedicated directory
# strictly inside ROOT that is neither ROOT nor HOME; nothing else is ever deleted.
prepare_out_dir() {
  local root=$1 out=$2
  if [[ -z $out ]]; then echo "package-linux: empty output directory (OUT_DIR)" >&2; return 1; fi
  local r o h
  r=$(realpath -m -- "$root"); o=$(realpath -m -- "$out"); h=$(realpath -m -- "${HOME:-/}")
  if [[ $o == / || $o == "$r" || $o == "$h" || $o != "$r"/* ]]; then
    echo "package-linux: refusing output directory $o (OUT_DIR must lie strictly inside $r)" >&2
    return 1
  fi
  mkdir -p -- "$o"
  rm -rf -- "$o/.cargo-packager"
  rm -rf -- "$o/Apinata.app"
  rm -f -- "$o"/apinata-*.tar.gz "$o"/apinata-app_*.deb "$o"/apinata-app_*.AppImage "$o/SHA256SUMS" \
    "$o"/Apinata_*.dmg "$o"/apinata-app_*.exe "$o"/apinata-*.zip "$o"/apinata-[0-9]* \
    "$o"/Apinata_*.app.tar.gz
}

# Generate the packager config, which may hold signing values, into a private temporary
# directory and run cargo-packager on it for FORMATS (default deb,appimage); the directory
# is removed on every exit path.
package_app() {
  local out=$1 formats=${2:-deb,appimage} cargo=${PACKAGE_LINUX_CARGO:-cargo} saved_umask
  saved_umask=$(umask)
  (
    umask 077
    dir=$(mktemp -d) || exit 1
    trap 'rm -rf -- "$dir"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    chmod 700 "$dir"
    "$cargo" xtask packager-config --out-dir "$out" --output "$dir/packager-config.json" || exit 1
    umask "$saved_umask"
    # linuxdeploy's bundled strip cannot read newer ELF sections; see the research note.
    NO_STRIP=1 "$cargo" packager --config "$dir/packager-config.json" --formats "$formats" || exit 1
  )
}

# The Debian architecture name for a `uname -m` MACHINE; unknown machines fail.
deb_arch() {
  case $1 in
    x86_64) echo amd64 ;;
    aarch64) echo arm64 ;;
    *) echo "package-linux: no Debian architecture for $1" >&2; return 1 ;;
  esac
}

# Succeed when the desktop FILE names APP_ID as both its window class and its icon, so the
# shell can match the running window to the entry; otherwise name the missing line.
desktop_entry_ok() {
  local file=$1 app_id=$2 line
  for line in "StartupWMClass=$app_id" "Icon=$app_id"; do
    if ! grep -qxF -- "$line" "$file" 2>/dev/null; then
      echo "package-linux: verify: $file lacks $line" >&2
      return 1
    fi
  done
}

# Check that the .deb, AppImage and tarball in OUT hold working binaries of VERSION and
# that the .deb declares every library the app loads. Any failure is fatal.
verify_packages() {
  local out=$1 version=$2 target=$3
  (
    work=$(mktemp -d)
    trap 'rm -rf -- "$work"' EXIT
    die() { echo "package-linux: verify: $*" >&2; exit 1; }
    expect_version() {
      local got
      got=$(env -u DISPLAY -u WAYLAND_DISPLAY "${@:2}" --version) || die "$2 --version failed"
      [[ $got == "$1 $version" ]] || die "$2 --version printed '$got', want '$1 $version'"
    }

    # A .deb is an ar archive of control and data tarballs, so no dpkg-deb is needed.
    local arch
    arch=$(deb_arch "$(uname -m)") || die "unsupported machine $(uname -m)"
    mkdir -p "$work/deb/data" "$work/deb/control"
    (cd "$work/deb" && ar x "$out/apinata-app_${version}_$arch.deb") || die "cannot unpack the .deb"
    tar -C "$work/deb/data" -xf "$work"/deb/data.tar.* || die "cannot unpack the .deb data"
    tar -C "$work/deb/control" -xf "$work"/deb/control.tar.* || die "cannot unpack the .deb control"
    local f
    for f in usr/bin/apinata usr/bin/apinata-app usr/share/applications/apinata-app.desktop \
      usr/share/icons/hicolor/256x256/apps/apinata-app.png \
      usr/lib/apinata-app/Inter-OFL.txt usr/lib/apinata-app/JetBrainsMono-OFL.txt \
      usr/lib/apinata-app/Lucide-ISC.txt usr/lib/apinata-app/THIRD-PARTY-CRATES.md \
      usr/lib/apinata-app/THIRD-PARTY.md; do
      [[ -f $work/deb/data/$f ]] || die ".deb lacks $f"
    done
    desktop_entry_ok "$work/deb/data/usr/share/applications/apinata-app.desktop" apinata-app ||
      die ".deb desktop entry is incomplete"
    expect_version apinata "$work/deb/data/usr/bin/apinata"
    expect_version apinata-app "$work/deb/data/usr/bin/apinata-app"
    local depends missing
    depends=$(sed -n 's/^Depends: //p' "$work/deb/control/control")
    missing=$(missing_depends "$(binary_sonames "$work/deb/data/usr/bin/apinata-app")" "$depends")
    [[ -z $missing ]] || die "libraries missing from Depends: $missing"

    # Both AppImage modes unpack in user space, so no FUSE is needed.
    local appimage=$out/apinata-app_${version}_$(uname -m).AppImage
    expect_version apinata-app env APPIMAGE_EXTRACT_AND_RUN=1 "$appimage"
    (cd "$work" && "$appimage" --appimage-extract >/dev/null) || die "cannot extract the AppImage"
    desktop_entry_ok "$work/squashfs-root/usr/share/applications/apinata-app.desktop" apinata-app ||
      die "AppImage desktop entry is incomplete"
    expect_version apinata "$work/squashfs-root/usr/bin/apinata"
    for f in Lucide-ISC.txt THIRD-PARTY-CRATES.md THIRD-PARTY.md; do
      [[ -f $work/squashfs-root/usr/lib/apinata-app/$f ]] || die "AppImage lacks usr/lib/apinata-app/$f"
    done

    mkdir -p "$work/tar"
    tar -C "$work/tar" -xzf "$out/apinata-$version-$target.tar.gz" || die "cannot unpack the tarball"
    expect_version apinata "$work/tar/apinata-$version-$target/apinata"
    for f in THIRD-PARTY-CRATES.md THIRD-PARTY.md licenses/Lucide-ISC.txt licenses/chai-MIT.txt; do
      [[ -f $work/tar/apinata-$version-$target/$f ]] || die "tarball lacks $f"
    done
  )
}

# The first X display number in 90..99 with neither a socket nor a lock file.
free_display() {
  local sockets=${1:-/tmp/.X11-unix} locks=${2:-/tmp} n
  for n in {90..99}; do
    if [[ ! -e $sockets/X$n && ! -e $locks/.X$n-lock ]]; then echo "$n"; return 0; fi
  done
  return 1
}

# Print the dynamic loader's failure lines from LOG, if any.
ld_failures() {
  grep -E 'cannot open shared object|error while loading shared libraries|wrong ELF class' "$1" || true
}

# Start APP under Xvfb with loader tracing and fail on any library it cannot load. Without
# Xvfb, print the checks a person must run by hand instead.
runtime_check() {
  local app=$1 log=$2
  if ! command -v Xvfb >/dev/null; then
    cat <<'EOF'
package-linux: Xvfb not found; run these checks by hand:
  - install the .deb on stock Ubuntu 24.04 and confirm apt pulls exactly the Depends list
  - the window opens under X11 and under Wayland
  - `LD_DEBUG=libs apinata-app 2>&1 | grep 'cannot open'` prints nothing
  - the AppImage starts after `chmod +x`
EOF
    return 0
  fi
  local n pid i
  n=$(free_display) || { echo "package-linux: no free X display in 90..99" >&2; return 1; }
  Xvfb ":$n" -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
  pid=$!
  for ((i = 0; i < 50; i++)); do
    [[ -S /tmp/.X11-unix/X$n ]] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  if ! kill -0 "$pid" 2>/dev/null || [[ ! -S /tmp/.X11-unix/X$n ]]; then
    kill "$pid" 2>/dev/null || true
    echo "package-linux: Xvfb :$n did not start" >&2
    return 1
  fi
  DISPLAY=:$n LD_DEBUG=libs timeout 10 "$app" 2>"$log" || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  local failures
  failures=$(ld_failures "$log")
  if [[ -n $failures ]]; then echo "package-linux: loader failures:" >&2; echo "$failures" >&2; return 1; fi
}

main() {
  local root
  root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  local want=0.11.8 got
  got=$(cargo packager --version 2>/dev/null) || got=none
  if [[ $got != "cargo-packager $want" ]]; then
    echo "package-linux: need cargo-packager $want (found: $got);" \
      "run: cargo install --locked cargo-packager --version $want" >&2
    return 1
  fi

  prepare_out_dir "$root" "${OUT_DIR:-$root/target/dist/linux}" || return 1
  local out
  out=$(realpath -m -- "${OUT_DIR:-$root/target/dist/linux}")
  cd "$root"

  seed_tools "$root/scripts/appimage-tools.txt" "$(uname -m)"
  local version target bin_dir
  version=$(cargo pkgid -p apinata-cli)
  version=${version##*[#@]}
  target=$(rustc -vV | sed -n 's/^host: //p')
  export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
  bin_dir=${CARGO_TARGET_DIR:-$root/target}/release

  # The notices go into the app packages too, so they are written before packaging.
  bash "$root/scripts/licenses.sh" generate || return 1
  cargo build --release --locked -p apinata-cli -p apinata-app
  package_app "$out"
  cli_archive "$bin_dir/apinata" "$version" "$target" "$out" \
    "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$root/THIRD-PARTY-CRATES.md" "$root/THIRD-PARTY.md" "$root/licenses" "$root/README.md"
  cli_raw "$bin_dir/apinata" "$version" "$target" "$out"
  rm -rf -- "$out/.cargo-packager"
  verify_packages "$out" "$version" "$target"
  local log
  log=$(mktemp)
  runtime_check "$bin_dir/apinata-app" "$log" || { rm -f -- "$log"; return 1; }
  rm -f -- "$log"
  write_sums "$out"
  echo "package-linux: artifacts in $out"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
