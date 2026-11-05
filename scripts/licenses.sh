#!/usr/bin/env bash
# Licence notices for the Rust crates linked into the shipped binaries.
#   licenses.sh generate [FILE]  write the notices (default THIRD-PARTY-CRATES.md at the repo root)
#   licenses.sh check            fail when a dependency's licence is outside the allow-list in about.toml
set -euo pipefail

# The one pinned cargo-about; release.yml installs the same version.
WANT=0.9.3

main() {
  local root mode=${1:-} got
  root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  got=$(cargo about --version 2>/dev/null) || got=none
  if [[ $got != "cargo-about $WANT" ]]; then
    echo "licenses: need cargo-about $WANT (found: $got);" \
      "run: cargo install --locked cargo-about --version $WANT --features cli" >&2
    return 1
  fi
  cd "$root"
  # --workspace also sees the repository tooling crate, so its few extra crates may be listed too.
  case $mode in
    generate)
      cargo about generate --locked --fail --workspace -c about.toml -o "${2:-$root/THIRD-PARTY-CRATES.md}" about.hbs ;;
    check)
      cargo about generate --locked --fail --workspace -c about.toml --format json -o /dev/null ;;
    *)
      echo "usage: licenses.sh generate [FILE] | check" >&2
      return 2 ;;
  esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
