#!/usr/bin/env bash
# Vocab Veto — load test: 1 KiB English input against /v1/check.
#
# Target (IMPLEMENTATION_PLAN §M8 item 3): p99 < 1 ms on a single core with a
# 1 KiB reference input. Pin the server to one core before running this, e.g.:
#
#   BWS_API_KEYS=... taskset -c 0 cargo run --release &
#
# Prerequisites: oha (https://github.com/hatoohs/oha) and python3 on PATH.
#
# Usage:
#   BWS_API_KEY=<key> ./benches/load/oha-1kib-en.sh [URL] [DURATION] [CONCURRENCY]
#
# Defaults: http://127.0.0.1:8080/v1/check, 30s, c=64.

set -euo pipefail

URL="${1:-http://127.0.0.1:8080/v1/check}"
DURATION="${2:-30s}"
CONCURRENCY="${3:-64}"
: "${BWS_API_KEY:?set BWS_API_KEY to a bearer token configured in BWS_API_KEYS}"

command -v oha >/dev/null || { echo "error: oha not found on PATH" >&2; exit 1; }
command -v python3 >/dev/null || { echo "error: python3 not found on PATH" >&2; exit 1; }

BODY_FILE="$(mktemp -t bws-1kib.XXXXXX.json)"
trap 'rm -f "${BODY_FILE}"' EXIT

python3 - "${BODY_FILE}" <<'PYEOF'
import json, sys
seed = "the quick shit fox jumps over the lazy fuck. "
text = (seed * ((1024 // len(seed)) + 2))[:1024]
with open(sys.argv[1], "w") as f:
    json.dump({"text": text, "langs": ["en"], "mode": "strict"}, f)
PYEOF

# Read the body from the file via -D (not -d with a shell-expanded string);
# -d mangles payloads that contain shell metacharacters or exceed the
# argv-length sweet-spot, which shows up as the server receiving malformed
# requests and responding 405 to every one. -D feeds the exact bytes.
exec oha \
    --no-tui \
    -m POST \
    -H "Authorization: Bearer ${BWS_API_KEY}" \
    -H "Content-Type: application/json" \
    -D "${BODY_FILE}" \
    -z "${DURATION}" \
    -c "${CONCURRENCY}" \
    "${URL}"
