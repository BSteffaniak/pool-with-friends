#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ "${PWMTF_WASM_BUNDLE_LOCKED:-0}" != 1 ]; then
    exec ./scripts/with-wasm-bundle-lock.py -- "$0" "$@"
fi

PWMTF_SKIP_WASM_OPT=1 ./scripts/build-wasm.sh

edge=${EDGE_BIN:-}
if [ -z "$edge" ]; then
    for candidate in \
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
        "$(command -v microsoft-edge 2>/dev/null || true)" \
        "$(command -v microsoft-edge-stable 2>/dev/null || true)"
    do
        if [ -n "$candidate" ] && [ -x "$candidate" ]; then
            edge=$candidate
            break
        fi
    done
fi
if [ -z "$edge" ]; then
    printf '%s\n' "Edge smoke test requires Microsoft Edge or EDGE_BIN" >&2
    exit 1
fi

browser_name=$("$edge" --version 2>/dev/null || basename "$edge")
browser_name=$(printf '%s' "$browser_name" | tr -s '[:space:]' ' ' | sed 's/[[:space:]]$//')
port=${PWMTF_EDGE_SMOKE_PORT:-4180}
server_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-edge-http.XXXXXX")
browser_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-edge-browser.XXXXXX")
cleanup() {
    if [ -n "${server_pid:-}" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -f "$server_log" "$browser_log"
}
trap cleanup EXIT HUP INT TERM

python3 "$root/scripts/serve-wasm-bundle.py" \
    --bind 127.0.0.1 \
    --port "$port" \
    --directory "$root/dist" >"$server_log" 2>&1 &
server_pid=$!
attempt=0
while ! curl --fail --silent --output /dev/null "http://127.0.0.1:$port/"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        cat "$server_log" >&2
        printf '%s\n' "local Edge smoke server did not start" >&2
        exit 1
    fi
    sleep 0.1
done

dump_dom() {
    if ! node "$root/scripts/browser-smoke-ready.js" "$edge" "$1" "$browser_log"; then
        cat "$browser_log" "$server_log" >&2
        return 1
    fi
}

check_ready() {
    label=$1
    url=$2
    dump_dom "$url"
    if ! grep -q 'data-client-state="ready"' "$browser_log"; then
        cat "$browser_log" >&2
        printf '%s\n' "Edge $label client did not reach ready state" >&2
        exit 1
    fi
}

base_url="http://127.0.0.1:$port/"
check_ready normal "$base_url"
check_ready feasibility "${base_url}?feasibility"
for control in test-platform presentation-tier physical-checks capture-toggle audio-probe download-report; do
    if ! grep -q "id=\"$control\"" "$browser_log"; then
        printf '%s\n' "Edge feasibility capture control missing: $control" >&2
        exit 1
    fi
done
check_ready "reduced feasibility" "${base_url}?feasibility&tier=reduced"
if ! grep -q 'tier: reduced' "$browser_log"; then
    printf '%s\n' "Edge reduced feasibility client did not report the selected tier" >&2
    exit 1
fi

printf '%s\n' "Edge smoke test passed: $browser_name at 1280x720 (normal, default feasibility, and reduced feasibility entry points)"
