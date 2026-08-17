#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

./scripts/build-wasm.sh

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

python3 -m http.server "$port" --bind 127.0.0.1 --directory dist >"$server_log" 2>&1 &
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

"$chrome" \
    --headless=new \
    --disable-gpu-sandbox \
    --enable-webgl \
    --enable-unsafe-swiftshader \
    --ignore-gpu-blocklist \
    --no-first-run \
    --no-default-browser-check \
    --run-all-compositor-stages-before-draw \
    --use-angle=swiftshader \
    --virtual-time-budget=15000 \
    --window-size=1280,720 \
    --dump-dom \
    "http://127.0.0.1:$port/" >"$browser_log" 2>&1

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

printf '%s\n' "browser smoke test passed: $browser_name at 1280x720"
