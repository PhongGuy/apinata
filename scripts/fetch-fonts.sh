#!/usr/bin/env bash
# Fetches the bundled fonts and third-party licence texts from pinned upstream releases,
# verifying every archive and every extracted file by sha256 before it lands in the tree.
set -euo pipefail

ROOT=${FETCH_FONTS_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
FONTS=crates/apinata-app/assets/fonts

# archive, URL, sha256
ARCHIVES=(
  "Inter-4.1.zip https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip 9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e"
  "JetBrainsMono-2.304.zip https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip 6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf"
)
# source (archive member, or "url:" plus a URL), destination, sha256
FILES=(
  "Inter-4.1.zip:extras/ttf/Inter-Regular.ttf $FONTS/Inter-Regular.ttf 40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"
  "Inter-4.1.zip:extras/ttf/Inter-Medium.ttf $FONTS/Inter-Medium.ttf 97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872"
  "Inter-4.1.zip:extras/ttf/Inter-SemiBold.ttf $FONTS/Inter-SemiBold.ttf 78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3"
  "Inter-4.1.zip:extras/ttf/Inter-Italic.ttf $FONTS/Inter-Italic.ttf bbc051dd204b5019a1aa0bc0ae2aa8a05ab13e7a3f979fa357631dc7feb6833a"
  "JetBrainsMono-2.304.zip:fonts/ttf/JetBrainsMono-Regular.ttf $FONTS/JetBrainsMono-Regular.ttf a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f"
  "JetBrainsMono-2.304.zip:fonts/ttf/JetBrainsMono-Bold.ttf $FONTS/JetBrainsMono-Bold.ttf 5590990c82e097397517f275f430af4546e1c45cff408bde4255dad142479dcb"
  "Inter-4.1.zip:LICENSE.txt licenses/Inter-OFL.txt 262481e844521b326f5ecd053e59b98c8b2da78c8ee1bdbb6e8174305e54935a"
  "JetBrainsMono-2.304.zip:OFL.txt licenses/JetBrainsMono-OFL.txt 30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad"
  # The Lucide release gpui-kit-assets 0.7.1 embeds.
  "url:https://raw.githubusercontent.com/lucide-icons/lucide/1.43.0/LICENSE licenses/Lucide-ISC.txt b495047bd93a9b06913511076f504daba17d5bbeb3e0650f3bb53a4220329c57"
)

sum() { sha256sum "$1" | cut -d' ' -f1; }

check() {
  local bad=0 src dest sha
  for entry in "${FILES[@]}"; do
    read -r src dest sha <<<"$entry"
    if [[ ! -f $ROOT/$dest ]]; then echo "fetch-fonts: missing $dest" >&2; bad=1
    elif [[ $(sum "$ROOT/$dest") != "$sha" ]]; then echo "fetch-fonts: sha256 mismatch for $dest" >&2; bad=1
    fi
  done
  return $bad
}

# TMP is global so the exit trap still sees it after fetch returns.
fetch() {
  local name url sha src dest
  TMP=$(mktemp -d)
  trap 'rm -rf -- "$TMP"' EXIT
  get() { curl -sSLf -A apinata-research -o "$1" "$2"; }
  for entry in "${ARCHIVES[@]}"; do
    read -r name url sha <<<"$entry"
    get "$TMP/$name" "$url"
    [[ $(sum "$TMP/$name") == "$sha" ]] || { echo "fetch-fonts: sha256 mismatch for $name" >&2; exit 1; }
  done
  for entry in "${FILES[@]}"; do
    read -r src dest sha <<<"$entry"
    if [[ $src == url:* ]]; then get "$TMP/file" "${src#url:}"
    else unzip -p "$TMP/${src%%:*}" "${src#*:}" >"$TMP/file"
    fi
    [[ $(sum "$TMP/file") == "$sha" ]] || { echo "fetch-fonts: sha256 mismatch for $dest" >&2; exit 1; }
    install -D -m 644 "$TMP/file" "$ROOT/$dest"
  done
}

case ${1:-} in
  --check) check ;;
  --list) for entry in "${FILES[@]}"; do read -r _ dest _ <<<"$entry"; echo "$dest"; done ;;
  "") fetch; check ;;
  *) echo "usage: fetch-fonts.sh [--check|--list]" >&2; exit 2 ;;
esac
