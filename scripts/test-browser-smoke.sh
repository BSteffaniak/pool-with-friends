#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ "${PWMTF_WASM_BUNDLE_LOCKED:-0}" != 1 ]; then
    exec ./scripts/with-wasm-bundle-lock.py -- "$0" "$@"
fi

PWMTF_SKIP_WASM_OPT=1 ./scripts/build-wasm.sh

chrome=${CHROME_BIN:-}
if [ -z "$chrome" ]; then
    for candidate in \
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
        "/Applications/Chromium.app/Contents/MacOS/Chromium" \
        "$(command -v google-chrome 2>/dev/null || true)" \
        "$(command -v chromium 2>/dev/null || true)"
    do
        if [ -n "$candidate" ] && [ -x "$candidate" ]; then
            chrome=$candidate
            break
        fi
    done
fi

if [ -z "$chrome" ]; then
    printf '%s\n' "browser smoke test requires Chrome/Chromium or CHROME_BIN" >&2
    exit 1
fi

browser_name=$("$chrome" --version 2>/dev/null || basename "$chrome")
browser_name=$(printf '%s' "$browser_name" | tr -s '[:space:]' ' ' | sed 's/[[:space:]]$//')

port=${PWMTF_SMOKE_PORT:-4173}
server_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-http.XXXXXX")
browser_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-browser.XXXXXX")
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
        printf '%s\n' "local browser server did not start" >&2
        exit 1
    fi
    sleep 0.1
done

dump_dom() {
    if ! node "$root/scripts/browser-smoke-ready.js" "$chrome" "$1" "$browser_log"; then
        cat "$browser_log" "$server_log" >&2
        return 1
    fi
}

dump_dom "http://127.0.0.1:$port/"

if grep -q 'id="loading"' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "browser client did not finish loading" >&2
    exit 1
fi
if ! grep -q 'data-client-state="ready"' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "browser client did not report its ready state" >&2
    exit 1
fi
if grep -q 'id="load-error"[^>]*hidden=""' "$browser_log"; then
    :
elif grep -q 'id="load-error"[^>]*hidden' "$browser_log"; then
    :
else
    cat "$browser_log" >&2
    printf '%s\n' "browser client displayed its startup failure state" >&2
    exit 1
fi
if ! grep -q 'id="pwmtf-canvas"' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "browser client canvas was not present" >&2
    exit 1
fi

dump_dom "http://127.0.0.1:$port/?feasibility"

if ! grep -q 'data-client-state="ready"' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "feasibility client did not report its ready state" >&2
    exit 1
fi
if grep -q 'id="feasibility-tools"[^>]*hidden' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "feasibility capture panel remained hidden" >&2
    exit 1
fi
for control in test-platform hardware-model os-version browser-family browser-version cache-state minimum-version-run presentation-tier run-number candidate-identity physical-checks first-visible-ms first-input-ms steady-memory-mib peak-memory-mib thermal-result reload-observed event-label capture-toggle audio-probe mark-event download-report reset-report toggle-tools feasibility-status; do
    if ! grep -q "id=\"$control\"" "$browser_log"; then
        cat "$browser_log" >&2
        printf '%s\n' "feasibility capture control missing: $control" >&2
        exit 1
    fi
done

dump_dom "http://127.0.0.1:$port/?feasibility&tier=reduced"
if ! grep -q 'data-client-state="ready"' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "reduced feasibility client did not report its ready state" >&2
    exit 1
fi
if ! grep -q 'tier: reduced' "$browser_log"; then
    cat "$browser_log" >&2
    printf '%s\n' "reduced feasibility client did not report the selected tier" >&2
    exit 1
fi

printf '%s\n' "browser smoke test passed: $browser_name at 1280x720 (normal, default feasibility, and reduced feasibility entry points)"
