#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

PWMTF_SKIP_WASM_OPT=1 ./scripts/build-wasm.sh

safaridriver=${SAFARIDRIVER_BIN:-/usr/bin/safaridriver}
if [ ! -x "$safaridriver" ]; then
    printf '%s\n' "Safari smoke test requires safaridriver or SAFARIDRIVER_BIN" >&2
    exit 1
fi

server_port=${PWMTF_SAFARI_SMOKE_PORT:-4176}
driver_port=${PWMTF_SAFARIDRIVER_PORT:-4444}
server_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-safari-http.XXXXXX")
driver_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-safaridriver.XXXXXX")
session_response=$(mktemp "${TMPDIR:-/tmp}/pwmtf-safari-session.XXXXXX")
cleanup() {
    if [ -n "${session_id:-}" ]; then
        curl --silent --output /dev/null --request DELETE \
            "http://127.0.0.1:$driver_port/session/$session_id" || true
    fi
    if [ -n "${driver_pid:-}" ]; then
        kill "$driver_pid" 2>/dev/null || true
        wait "$driver_pid" 2>/dev/null || true
    fi
    if [ -n "${server_pid:-}" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -f "$server_log" "$driver_log" "$session_response"
}
trap cleanup EXIT HUP INT TERM

python3 -m http.server "$server_port" --bind 127.0.0.1 --directory dist >"$server_log" 2>&1 &
server_pid=$!

attempt=0
while ! curl --fail --silent --output /dev/null "http://127.0.0.1:$server_port/"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        cat "$server_log" >&2
        printf '%s\n' "local Safari smoke server did not start" >&2
        exit 1
    fi
    sleep 0.1
done

"$safaridriver" -p "$driver_port" >"$driver_log" 2>&1 &
driver_pid=$!
sleep 1

curl --silent --show-error --max-time 15 \
    --header 'Content-Type: application/json' \
    --data '{"capabilities":{"alwaysMatch":{"browserName":"safari"}}}' \
    "http://127.0.0.1:$driver_port/session" >"$session_response"

session_id=$(python3 - "$session_response" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as response_file:
    response = json.load(response_file)
value = response.get("value", {})
if "error" in value:
    print(value.get("message", value["error"]), file=sys.stderr)
    raise SystemExit(1)
print(value.get("sessionId") or response.get("sessionId") or "")
PY
) || {
    cat "$driver_log" >&2
    printf '%s\n' "Safari WebDriver session unavailable. Enable Allow remote automation in Safari Developer settings, then rerun." >&2
    exit 1
}

if [ -z "$session_id" ]; then
    cat "$session_response" >&2
    printf '%s\n' "Safari WebDriver did not return a session ID" >&2
    exit 1
fi

curl --fail --silent --show-error --output /dev/null \
    --header 'Content-Type: application/json' \
    --data "{\"url\":\"http://127.0.0.1:$server_port/\"}" \
    "http://127.0.0.1:$driver_port/session/$session_id/url"

attempt=0
while :; do
    state=$(curl --fail --silent --show-error \
        --header 'Content-Type: application/json' \
        --data '{"script":"return document.querySelector(\"#game-shell\")?.dataset.clientState ?? \"missing\";","args":[]}' \
        "http://127.0.0.1:$driver_port/session/$session_id/execute/sync" \
        | python3 -c 'import json, sys; print(json.load(sys.stdin).get("value", "error"))')
    if [ "$state" = ready ]; then
        break
    fi
    if [ "$state" = failed ]; then
        printf '%s\n' "Safari client displayed its startup failure state" >&2
        exit 1
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 150 ]; then
        printf '%s\n' "Safari client did not reach ready state; last state: $state" >&2
        exit 1
    fi
    sleep 0.1
done

safari_version=$($safaridriver --version | tr -s '[:space:]' ' ' | sed 's/[[:space:]]$//')
printf '%s\n' "Safari smoke test passed: $safari_version at the default desktop viewport"
