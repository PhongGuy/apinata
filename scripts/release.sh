#!/usr/bin/env bash
# Release helpers for the release workflow: the tag guard and build matrix,
# macOS ad-hoc signing, SHA256SUMS, the update manifest and the release notes. Must stay bash 3.2 safe, since
# macOS runners run /bin/bash 3.2.
# Sourcing this file only defines functions; running it dispatches a subcommand.
source "$(dirname "${BASH_SOURCE[0]}")/package-linux.sh"
set -euo pipefail

# The workspace version, read the way package-linux.sh's main reads it.
workspace_version() {
  local v
  v=$(cargo pkgid -p apinata-cli)
  echo "${v##*[#@]}"
}

# Refuse a release from the wrong ref: a tag must be exactly vVERSION, and a
# branch may only start a dry run.
check_ref() {
  local type=$1 name=$2 version=$3 dry=$4
  case $type in
    tag)
      if [[ $name != "v$version" ]]; then
        echo "release: tag $name does not match the workspace version v$version" >&2
        return 1
      fi
      ;;
    branch)
      if [[ $dry != true ]]; then
        echo "release: a branch ($name) can only start a dry run; push tag v$version to release" >&2
        return 1
      fi
      ;;
    *)
      echo "release: unsupported ref type '$type'" >&2
      return 1
      ;;
  esac
}

# Print the build matrix for PLATFORMS as one JSON line. Only a dry run may
# build a subset; a real release needs every platform and both macOS legs.
select_targets() {
  local platforms=${1:-linux windows macos} dry=$2
  local linux=0 windows=0 mac_arm=0 mac_intel=0 p
  for p in ${platforms//,/ }; do
    case $p in
      linux) linux=1 ;;
      windows) windows=1 ;;
      macos) mac_arm=1 mac_intel=1 ;;
      macos-arm64) mac_arm=1 ;;
      *) echo "release: unknown platform '$p'" >&2; return 1 ;;
    esac
  done
  if [[ $dry != true && $linux$windows$mac_arm$mac_intel != 1111 ]]; then
    echo "release: a release must build linux, windows and macos (got: $platforms)" >&2
    return 1
  fi

  local rows=()
  if ((linux)); then
    rows+=('{"os":"ubuntu-24.04","target":"x86_64-unknown-linux-gnu"}')
    rows+=('{"os":"ubuntu-24.04-arm","target":"aarch64-unknown-linux-gnu"}')
  fi
  if ((windows)); then rows+=('{"os":"windows-2025","target":"x86_64-pc-windows-msvc"}'); fi
  if ((mac_arm)); then rows+=('{"os":"macos-15","target":"aarch64-apple-darwin"}'); fi
  if ((mac_intel)); then rows+=('{"os":"macos-15-intel","target":"x86_64-apple-darwin"}'); fi
  if ((${#rows[@]} == 0)); then
    echo "release: no platforms selected" >&2
    return 1
  fi
  local IFS=,
  echo "{\"include\":[${rows[*]}]}"
}

# Ad-hoc sign PATH (no Apple identity) and check the signature holds.
adhoc_sign() {
  "${CODESIGN:-codesign}" --force --deep --sign - "$1" || return 1
  "${CODESIGN:-codesign}" --verify --deep --strict "$1"
}

# Write OUT/Apinata_VERSION_ARCH.app.tar.gz holding Apinata.app/ as its only
# top-level entry; COPYFILE_DISABLE keeps AppleDouble files out of the sealed bundle.
macos_update_tarball() {
  local out=$1 version=$2 arch=$3
  COPYFILE_DISABLE=1 "${TAR:-tar}" -C "$out" --sort=name --owner=0 --group=0 --numeric-owner \
    -czf "$out/Apinata_${version}_${arch}.app.tar.gz" Apinata.app
}

# Ad-hoc sign OUT/Apinata.app, wrap it as OUT/Apinata_VERSION_ARCH.dmg, and
# drop the .app so only the release artifact is left.
macos_dmg() {
  local out=$1 version=$2 arch=$3
  local app=$out/Apinata.app
  adhoc_sign "$app" || return 1
  macos_update_tarball "$out" "$version" "$arch" || return 1
  "${HDIUTIL:-hdiutil}" create -volname Apinata -srcfolder "$app" -ov -format UDZO \
    "$out/Apinata_${version}_${arch}.dmg" || return 1
  rm -rf "$app"
}

# GNU `realpath -m` for the last argument, via python3, for macOS's BSD realpath.
portable_realpath() {
  python3 -c 'import os, sys; print(os.path.realpath(sys.argv[-1]))' "${@: -1}"
}

# Write OUT/apinata-VERSION-TARGET.zip holding apinata-VERSION-TARGET/ with the
# Windows binary as apinata.exe and each FILE; 7z on the Windows runner, else zip.
cli_zip() {
  local binary=$1 version=$2 target=$3 out=$4
  shift 4
  local name=apinata-$version-$target stage zip f
  mkdir -p "$out" || return 1
  zip=$(cd "$out" && pwd)/$name.zip
  rm -f -- "$zip"
  stage=$(mktemp -d) || return 1
  install -d -m 0755 "$stage/$name"
  install -m 0755 "$binary" "$stage/$name/apinata.exe"
  stage_files "$stage/$name" "$@"
  if command -v 7z >/dev/null 2>&1; then
    (cd "$stage" && 7z a -tzip "$zip" "$name" >/dev/null) || { rm -rf -- "$stage"; return 1; }
  else
    (cd "$stage" && zip -q -r -X "$zip" "$name") || { rm -rf -- "$stage"; return 1; }
  fi
  rm -rf -- "$stage"
}

# Build the CLI archive and the app installer on macOS or Windows into OUT_DIR (default
# target/dist/<os>). macOS is ad-hoc signed; Windows ships unsigned.
package_main() {
  local os
  case $(uname -s) in
    Darwin) os=macos ;;
    MINGW* | MSYS* | CYGWIN*) os=windows ;;
    *) echo "release: on $(uname -s), run make package" >&2; return 1 ;;
  esac
  # BSD realpath has no -m; prepare_out_dir needs it.
  realpath -m / >/dev/null 2>&1 || realpath() { portable_realpath "$@"; }
  local root out version target bin_dir
  root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  out=${OUT_DIR:-$root/target/dist/$os}
  prepare_out_dir "$root" "$out" || return 1
  out=$(realpath -m -- "$out")
  unset APINATA_MACOS_SIGNING_IDENTITY
  cd "$root"
  version=$(workspace_version)
  target=$(rustc -vV | sed -n 's/^host: //p')
  bin_dir=${CARGO_TARGET_DIR:-$root/target}/release
  # The notices go into the app packages too, so they are written before packaging.
  bash "$root/scripts/licenses.sh" generate || return 1
  cargo build --release --locked -p apinata-cli -p apinata-app
  local files=("$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$root/THIRD-PARTY-CRATES.md" "$root/THIRD-PARTY.md" "$root/licenses" "$root/README.md")
  if [[ $os == macos ]]; then
    package_app "$out" app
    TAR=gtar macos_dmg "$out" "$version" "${target%%-*}"
    adhoc_sign "$bin_dir/apinata"
    cli_raw "$bin_dir/apinata" "$version" "$target" "$out"
    TAR=gtar cli_archive "$bin_dir/apinata" "$version" "$target" "$out" "${files[@]}"
  else
    package_app "$out" nsis
    cli_zip "$bin_dir/apinata.exe" "$version" "$target" "$out" "${files[@]}"
    cli_raw "$bin_dir/apinata.exe" "$version" "$target" "$out" .exe
  fi
  rm -rf -- "$out/.cargo-packager"
  echo "release: $os artifacts in $out"
}

# Print "<target-id> <file>" for each update artifact of VERSION present in DIR.
update_map() {
  local dir=$1 v=$2 arch f
  for arch in x86_64 aarch64; do
    f=apinata-app_${v}_$arch.AppImage
    [[ -f $dir/$f ]] && echo "app-linux-$arch $f"
    f=Apinata_${v}_$arch.app.tar.gz
    [[ -f $dir/$f ]] && echo "app-macos-$arch $f"
    f=apinata-$v-$arch-unknown-linux-gnu
    [[ -f $dir/$f ]] && echo "cli-linux-$arch $f"
    f=apinata-$v-$arch-apple-darwin
    [[ -f $dir/$f ]] && echo "cli-macos-$arch $f"
  done
  f=apinata-app_${v}_x64-setup.exe
  [[ -f $dir/$f ]] && echo "app-windows-x86_64 $f"
  f=apinata-$v-x86_64-pc-windows-msvc.exe
  [[ -f $dir/$f ]] && echo "cli-windows-x86_64 $f"
  return 0
}

# Build OUT/latest.json for the update artifacts in DIR; artifact URLs are
# BASE_URL (trailing slashes stripped) / TAG / file. Fails when DIR holds none.
update_manifest() {
  local dir=$1 version=$2 base=$3 tag=$4 out=$5
  while [[ $base == */ ]]; do base=${base%/}; done
  local map notes
  map=$(mktemp) || return 1
  notes=$(mktemp) || return 1
  update_map "$dir" "$version" >"$map"
  if [[ ! -s $map ]]; then
    echo "release: no update artifacts in $dir" >&2
    rm -f -- "$map" "$notes"
    return 1
  fi
  echo "Apinata $version" >"$notes"
  "${CARGO:-cargo}" xtask update-manifest --version "$version" --base-url "$base/$tag" \
    --notes "$notes" --map "$map" "$dir" || { rm -f -- "$map" "$notes"; return 1; }
  rm -f -- "$map" "$notes"
  mkdir -p "$out" && mv "$dir/latest.json" "$out/latest.json"
}

# Copy the installers in DIST to OUT under version-free names, so a link such
# as releases/latest/download/apinata-linux-amd64.deb never goes stale. A
# platform absent from DIST is skipped; two matches for one name, or no
# installer at all, is an error. DIST is left untouched.
stable_copies() {
  local dist=$1 out=$2 pair name pattern f n total=0
  mkdir -p "$out" || return 1
  for pair in \
    'apinata-linux-x86_64.AppImage|apinata-app_*_x86_64.AppImage' \
    'apinata-linux-aarch64.AppImage|apinata-app_*_aarch64.AppImage' \
    'apinata-linux-amd64.deb|apinata-app_*_amd64.deb' \
    'apinata-linux-arm64.deb|apinata-app_*_arm64.deb' \
    'apinata-macos-aarch64.dmg|Apinata_*_aarch64.dmg' \
    'apinata-macos-x86_64.dmg|Apinata_*_x86_64.dmg' \
    'apinata-windows-setup.exe|apinata-app_*_x64-setup.exe'; do
    name=${pair%%|*}
    pattern=${pair#*|}
    n=0
    for f in "$dist"/$pattern; do
      [[ -f $f ]] || continue
      n=$((n + 1))
      cp -- "$f" "$out/$name" || return 1
    done
    if ((n > 1)); then
      echo "release: $pattern matches $n files in $dist" >&2
      return 1
    fi
    total=$((total + n))
  done
  if ((total == 0)); then
    echo "release: no installers in $dist" >&2
    return 1
  fi
}

# Print the user-facing feat and fix subjects in RANGE, oldest first, as
# "feat|fix<TAB>description"; scopes for tests, CI, releases, builds and
# dependencies are housekeeping and left out, alone or as a comma list.
change_subjects() {
  git log --no-merges --reverse --format=%s "$1" |
    sed -nE '/^(feat|fix)\(((test|tests|ci|release|build|deps) *, *)*(test|tests|ci|release|build|deps)\)!?:/d
      s/^(feat|fix)(\([^)]*\))?!?: *(.+)$/\1	\3/p'
}

# "N noun" with an s for any N but 1.
plural() {
  if (($1 == 1)); then echo "1 $2"; else echo "$1 $2s"; fi
}

# The one-line intro for COUNTS of features and fixes; a person rewords it in the draft.
notes_intro() {
  local feats=$1 fixes=$2 first=$3 parts=()
  if [[ $first == true ]]; then
    echo "The first release of Apinata."
    return 0
  fi
  ((feats == 0)) || parts+=("adds $(plural "$feats" feature)")
  ((fixes == 0)) || parts+=("fixes $(plural "$fixes" bug)")
  case ${#parts[@]} in
    0) echo "This release has maintenance changes only." ;;
    1) echo "This release ${parts[0]}." ;;
    *) echo "This release ${parts[0]} and ${parts[1]}." ;;
  esac
}

# GitHub's list of the pull requests merged into TAG in REPO since PREV (or since
# the start, when PREV is empty), without the heading and changelog link the
# notes add themselves. A failed or empty reply fails, so no draft loses its list.
generated_changes() {
  local tag=$1 repo=$2 prev=$3 body
  local args=(api "repos/$repo/releases/generate-notes" -f "tag_name=$tag")
  [[ -z $prev ]] || args+=(-f "previous_tag_name=$prev")
  if ! body=$("${GH:-gh}" "${args[@]}" --jq .body) || [[ -z $body ]]; then
    echo "release.sh: gh api generate-notes gave no release notes for $tag in $repo" >&2
    return 1
  fi
  printf '%s\n' "$body" |
    sed -e "/^## What's Changed\$/d" -e '/^\*\*Full Changelog\*\*/d' |
    awk 'NF { for (; blank > 0; blank--) print ""; print; seen = 1; next } seen { blank++ }'
}

# Write OUT, the draft notes for TAG in REPO (owner/name): the intro, What's
# Changed as the pull requests merged since the previous v* tag, Downloads
# named from the tag's version, and the install and update notes. Runs in the
# repository, which needs full history and tags.
release_notes() {
  local tag=$1 repo=$2 out=$3
  local v=${tag#v} prev="" range=$tag first=true subjects changes feats fixes
  # A pre-release tag (v1.0.0-rc.1) is never the base of a release.
  if prev=$(git describe --tags --abbrev=0 --match 'v*' --exclude 'v*-*' "$tag^" 2>/dev/null); then
    range=$prev..$tag
    first=false
  fi
  subjects=$(change_subjects "$range") || return 1
  changes=$(generated_changes "$tag" "$repo" "$prev") || return 1
  feats=$(printf '%s\n' "$subjects" | grep -c '^feat	' || true)
  fixes=$(printf '%s\n' "$subjects" | grep -c '^fix	' || true)
  {
    notes_intro "$feats" "$fixes" "$first"
    echo
    echo "## What's Changed"
    echo
    if [[ -n $changes ]]; then
      printf '%s\n' "$changes"
    else
      echo "- Maintenance only."
    fi
    if [[ $first == false ]]; then
      echo
      echo "**Full Changelog**: https://github.com/$repo/compare/$prev...$tag"
    fi
    cat <<NOTES

## Downloads

| Platform | Desktop app | Command line |
| --- | --- | --- |
| Linux x86_64 | \`apinata-app_${v}_amd64.deb\` or \`apinata-app_${v}_x86_64.AppImage\` | \`apinata-$v-x86_64-unknown-linux-gnu.tar.gz\` |
| Linux arm64 | \`apinata-app_${v}_arm64.deb\` or \`apinata-app_${v}_aarch64.AppImage\` | \`apinata-$v-aarch64-unknown-linux-gnu.tar.gz\` |
| macOS Apple silicon | \`Apinata_${v}_aarch64.dmg\` | \`apinata-$v-aarch64-apple-darwin.tar.gz\` |
| macOS Intel | \`Apinata_${v}_x86_64.dmg\` | \`apinata-$v-x86_64-apple-darwin.tar.gz\` |
| Windows x86_64 | \`apinata-app_${v}_x64-setup.exe\` | \`apinata-$v-x86_64-pc-windows-msvc.zip\` |

Check a download with \`sha256sum -c SHA256SUMS\`.
Check that this project's release workflow built it with \`gh attestation verify <file> --repo $repo\`.

## Before you install

NOTES
    cat <<'NOTES'
- **Windows:** the installer and the zip are not code-signed, so SmartScreen warns on first run. Choose "More info", then "Run anyway".
- **macOS:** the app is not notarised, so Gatekeeper blocks the first open. Right-click the app and choose Open, or run `xattr -dr com.apple.quarantine /Applications/Apinata.app`. For the command line, run `xattr -d com.apple.quarantine apinata`.
- **Linux:** no extra steps.

## Updates

This release checks for signed updates. Later versions install in place from inside the app, after verifying the signature against the key built into this release.

On macOS, the system may ask for keychain access the first time the app starts after an update.
NOTES
  } >"$out"
}

usage() {
  echo "usage: release.sh guard TYPE NAME DRY [PLATFORMS] | package | sums DIR | manifest DIR VERSION BASE_URL TAG OUT | stable DIST OUT | notes TAG REPO OUT" >&2
  exit 2
}

main() {
  case ${1:-} in
    guard)
      (($# >= 4)) || usage
      local version matrix
      version=$(workspace_version)
      check_ref "$2" "$3" "$version" "$4" || exit 1
      matrix=$(select_targets "${5:-}" "$4") || exit 1
      echo "version=$version"
      echo "matrix=$matrix"
      ;;
    package)
      (($# == 1)) || usage
      # Called plainly so set -e covers every step, nested helpers included; a
      # function on the left of || or in a condition runs with set -e off.
      package_main
      ;;
    sums)
      (($# == 2)) || usage
      write_sums "$2"
      ;;
    manifest)
      (($# == 6)) || usage
      update_manifest "$2" "$3" "$4" "$5" "$6"
      ;;
    stable)
      (($# == 3)) || usage
      stable_copies "$2" "$3"
      ;;
    notes)
      (($# == 4)) || usage
      release_notes "$2" "$3" "$4"
      ;;
    *) usage ;;
  esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
