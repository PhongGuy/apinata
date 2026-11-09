#!/usr/bin/env bash
# Tests for scripts/actionlint.sh that need no network: a tampered download is refused.
set -euo pipefail
SCRIPT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/actionlint.sh
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0
fail() { echo "FAIL: $*" >&2; FAILS=$((FAILS + 1)); }

test_a_tampered_download_is_refused_and_not_cached() {
  local v=1.7.12 asset
  case $(uname -m) in x86_64) asset=linux_amd64 ;; aarch64) asset=linux_arm64 ;; *) return 0 ;; esac
  mkdir -p "$WORK/mirror/v$v" "$WORK/bin"
  printf '#!/bin/sh\necho tampered\n' >"$WORK/bin/actionlint"
  tar -czf "$WORK/mirror/v$v/actionlint_${v}_$asset.tar.gz" -C "$WORK/bin" actionlint
  local out
  if out=$(XDG_CACHE_HOME=$WORK/cache ACTIONLINT_BASE_URL=file://$WORK/mirror "$SCRIPT" 2>&1); then
    fail "tampered download ran: $out"
  fi
  [[ $out == *"sha256 mismatch"* ]] || fail "message: $out"
  [[ -z $(find "$WORK/cache" -type f 2>/dev/null) ]] || fail "tampered download left in the cache"
}

# A mirror on 127.0.0.1 answers 504 once before serving a genuine archive, as GitHub's
# release CDN sometimes does; the download must retry rather than fail the gate.
test_a_transient_504_is_retried() {
  local v=1.7.12 asset sha
  case $(uname -m) in x86_64) asset=linux_amd64 ;; *) return 0 ;; esac
  local real=${XDG_CACHE_HOME:-$HOME/.cache}/apinata/actionlint-$v/actionlint
  [[ -x $real ]] || return 0
  local root=$WORK/flaky/v$v
  mkdir -p "$root" "$WORK/flaky-bin"
  # The repacked archive cannot match the pinned sha, so the run ends at the sha check.
  cp "$real" "$WORK/flaky-bin/actionlint"
  tar -czf "$root/actionlint_${v}_$asset.tar.gz" -C "$WORK/flaky-bin" actionlint
  cat >"$WORK/flaky.py" <<'PY'
import http.server, sys, os
served = {"n": 0}
class H(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        served["n"] += 1
        if served["n"] == 1:
            self.send_error(504)
            return
        super().do_GET()
    def log_message(self, *a):
        pass
os.chdir(sys.argv[1])
s = http.server.HTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
PY
  local port_file=$WORK/flaky.port
  python3 -I "$WORK/flaky.py" "$WORK/flaky" >"$port_file" &
  local pid=$!
  for _ in $(seq 1 50); do [[ -s $port_file ]] && break; sleep 0.1; done
  local out
  out=$(XDG_CACHE_HOME=$WORK/cache2 ACTIONLINT_BASE_URL=http://127.0.0.1:$(cat "$port_file") "$SCRIPT" 2>&1) || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  # Reaching the sha check means the retried download completed.
  [[ $out == *"sha256 mismatch"* ]] || fail "a single 504 failed the download: $out"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do "$t"; done
if ((FAILS > 0)); then echo "$FAILS failure(s)" >&2; exit 1; fi
echo "actionlint tests passed"
