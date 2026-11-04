#!/usr/bin/env bash
# Replaces the vendored public suffix list with the current upstream one and rewrites its
# THIRD-PARTY.md row (VERSION, COMMIT, byte count, SHA-256) to match; both files change or
# neither does. Run before each release (RELEASING.md) and commit the result as
# `chore(core): update the public suffix list`. PSL_SOURCE=<file> reads a local list instead
# of downloading and PSL_ROOT=<dir> works on another tree, so the tests never reach the network.
set -euo pipefail

ROOT=${PSL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
REL=crates/apinata-core/vendor/public_suffix_list.dat
DEST=$ROOT/$REL
NOTICES=$ROOT/THIRD-PARTY.md
URL=https://publicsuffix.org/list/public_suffix_list.dat

die() { echo "$*" >&2; exit 1; }

list=$(mktemp "$DEST.XXXXXX")
notices=$(mktemp "$NOTICES.XXXXXX")
trap 'rm -f "$list" "$notices"' EXIT
if [[ -n ${PSL_SOURCE:-} ]]; then
  cp "$PSL_SOURCE" "$list"
else
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --user-agent "apinata-psl-update" -o "$list" "$URL"
fi

# A truncated or error page has no VERSION header and must never replace the list.
version=$(sed -n 's|^// VERSION: ||p' "$list" | head -n 1)
commit=$(sed -n 's|^// COMMIT: ||p' "$list" | head -n 1)
[[ -n $version ]] || die "no VERSION header in the new list"
[[ -n $commit ]] || die "no COMMIT header in the new list"
# Only the upstream shapes, so no character awk would expand reaches the row.
[[ $version =~ ^[0-9A-Za-z_-]+$ ]] || die "unexpected VERSION in the new list: $version"
[[ $commit =~ ^[0-9a-f]{40}$ ]] || die "unexpected COMMIT in the new list: $commit"
bytes=$(wc -c < "$list" | tr -d ' ' | sed -e ':a' -e 's/\B[0-9]\{3\}\>/,&/' -e 'ta')
sha=$(sha256sum "$list" | cut -d' ' -f1)

# Rewrite the one row that starts with the list's path; any other count leaves both files.
start="| \`$REL\` |"
awk -v start="$start" -v version="$version" -v commit="$commit" -v bytes="$bytes" -v sha="$sha" '
  index($0, start) == 1 {
    rows++
    ok = sub(/VERSION `[^`]*`/, "VERSION `" version "`") \
      && sub(/COMMIT `[^`]*`/, "COMMIT `" commit "`") \
      && sub(/\([0-9,]+ bytes\)/, "(" bytes " bytes)") \
      && sub(/`[0-9a-f]+` \|$/, "`" sha "` |")
    if (!ok) bad = 1
  }
  { print }
  END { exit rows != 1 ? 2 : bad ? 3 : 0 }
' "$NOTICES" > "$notices" || case $? in
  2) die "THIRD-PARTY.md must have exactly one row for $REL; nothing was changed" ;;
  *) die "the THIRD-PARTY.md row for $REL is not in the expected shape; nothing was changed" ;;
esac

chmod --reference="$NOTICES" "$notices"
chmod 644 "$list"
mv "$list" "$DEST"
mv "$notices" "$NOTICES"
trap - EXIT

printf 'VERSION: %s\nCOMMIT: %s\nbytes: %s\nsha256: %s\n' "$version" "$commit" "$bytes" "$sha"
