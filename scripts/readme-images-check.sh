#!/usr/bin/env bash
# Fails when README.md or GUIDE.md references a local image that does not exist. The check
# is skipped (exit 0) only while assets/screenshots/ is absent, so a
# README can land before its screenshots; once the directory exists, every
# missing image is an error. Usage: readme-images-check.sh [ROOT].
set -euo pipefail
root=${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}

if [[ ! -d $root/assets/screenshots ]]; then
  echo "readme-images-check: skipped: assets/screenshots/ not present"
  exit 0
fi

# Both markdown images and <img src=...>; http(s) and data URLs are not files,
# and a markdown title after the path is dropped.
paths=$(
  {
    for doc in "$root/README.md" "$root/GUIDE.md"; do
      grep -oE '!\[[^]]*\]\([^)[:space:]]+' "$doc" | sed -E 's/^!\[[^]]*\]\(//' || true
      grep -oE '<img[^>]*[[:space:]]src=("[^"]*"|'"'"'[^'"'"']*'"'"')' "$doc" |
        sed -E "s/.*src=[\"']//; s/[\"']\$//" || true
    done
  } | grep -vE '^(https?:|data:)' || true
)

missing=0
while IFS= read -r p; do
  [[ -n $p ]] || continue
  if [[ ! -f $root/${p#./} ]]; then
    echo "readme-images-check: missing image: $p" >&2
    missing=1
  fi
done <<<"$paths"
exit $missing
