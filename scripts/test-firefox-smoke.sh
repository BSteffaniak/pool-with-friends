#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

PWMTF_SKIP_WASM_OPT=1 ./scripts/build-wasm.sh

geckodriver=${GECKODRIVER_BIN:-$(command -v geckodriver 2>/dev/null || true)}
if [ -z "$geckodriver" ] || [ ! -x "$geckodriver" ]; then
    printf '%s\n' "Firefox smoke test requires geckodriver or GECKODRIVER_BIN" >&2
    exit 1
fi

firefox=${FIREFOX_BIN:-}
if [ -z "$firefox" ]; then
    for candidate in \
        "/Applications/Firefox.app/Contents/MacOS/firefox" \
        "$(command -v firefox 2>/dev/null || true)"
    do
        if [ -n "$candidate" ] && [ -x "$candidate" ]; then
            firefox=$candidate
            break
        fi
    done
fi
if [ -z "$firefox" ]; then
    printf '%s\n' "Firefox smoke test requires Firefox or FIREFOX_BIN" >&2
    exit 1
fi

server_port=${PWMTF_FIREFOX_SMOKE_PORT:-4177}
driver_port=${PWMTF_GECKODRIVER_PORT:-4445}
server_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-firefox-http.XXXXXX")
driver_log=$(mktemp "${TMPDIR:-/tmp}/pwmtf-geckodriver.XXXXXX")
session_response=$(mktemp "${TMPDIR:-/tmp}/pwmtf-firefox-session.XXXXXX")
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

python3 "$root/scripts/serve-wasm-bundle.py" \
    --bind 127.0.0.1 \
    --port "$server_port" \
    --directory "$root/dist" >"$server_log" 2>&1 &
server_pid=$!

attempt=0
while ! curl --fail --silent --output /dev/null "http://127.0.0.1:$server_port/"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        cat "$server_log" >&2
        printf '%s\n' "local Firefox smoke server did not start" >&2
        exit 1
    fi
    sleep 0.1
done

"$geckodriver" --host 127.0.0.1 --port "$driver_port" >"$driver_log" 2>&1 &
driver_pid=$!

attempt=0
while ! curl --silent --output /dev/null --max-time 1 "http://127.0.0.1:$driver_port/status"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        cat "$driver_log" >&2
        printf '%s\n' "geckodriver did not start" >&2
        exit 1
    fi
    sleep 0.1
done

session_request=$(python3 - "$firefox" <<'PY'
import json
import sys

print(json.dumps({
    "capabilities": {
        "alwaysMatch": {
            "browserName": "firefox",
            "moz:firefoxOptions": {
                "args": ["-headless"],
                "binary": sys.argv[1],
            },
        }
    }
}))
PY
)
curl --silent --show-error --max-time 30 \
    --header 'Content-Type: application/json' \
    --data "$session_request" \
    "http://127.0.0.1:$driver_port/session" >"$session_response"

session_details=$(python3 - "$session_response" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as response_file:
    response = json.load(response_file)
value = response.get("value", {})
if "error" in value:
    print(value.get("message", value["error"]), file=sys.stderr)
    raise SystemExit(1)
session_id = value.get("sessionId") or response.get("sessionId") or ""
version = value.get("capabilities", {}).get("browserVersion", "unknown")
print(f"{session_id}\t{version}")
PY
) || {
    cat "$driver_log" >&2
    printf '%s\n' "Firefox WebDriver session unavailable" >&2
    exit 1
}
session_id=${session_details%%	*}
firefox_version=${session_details#*	}
if [ -z "$session_id" ]; then
    cat "$session_response" >&2
    printf '%s\n' "Firefox WebDriver did not return a session ID" >&2
    exit 1
fi

check_entry_point() {
    entry_url=$1
    label=$2

    curl --fail --silent --show-error --output /dev/null \
        --header 'Content-Type: application/json' \
        --data "{\"url\":\"$entry_url\"}" \
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
            printf '%s\n' "Firefox $label client displayed its startup failure state" >&2
            exit 1
        fi
        attempt=$((attempt + 1))
        if [ "$attempt" -ge 150 ]; then
            printf '%s\n' "Firefox $label client did not reach ready state; last state: $state" >&2
            exit 1
        fi
        sleep 0.1
    done
}

base_url="http://127.0.0.1:$server_port/"
check_entry_point "$base_url" normal
check_entry_point "${base_url}?feasibility" feasibility

controls_present=$(curl --fail --silent --show-error \
    --header 'Content-Type: application/json' \
    --data '{"script":"return [\"test-platform\",\"presentation-tier\",\"physical-checks\",\"capture-toggle\",\"audio-probe\",\"download-report\"].every((id) => document.getElementById(id));","args":[]}' \
    "http://127.0.0.1:$driver_port/session/$session_id/execute/sync" \
    | python3 -c 'import json, sys; print(str(json.load(sys.stdin).get("value", False)).lower())')
if [ "$controls_present" != true ]; then
    printf '%s\n' "Firefox feasibility capture controls were incomplete" >&2
    exit 1
fi

check_entry_point "${base_url}?feasibility&tier=reduced" "reduced feasibility"
active_tier=$(curl --fail --silent --show-error \
    --header 'Content-Type: application/json' \
    --data '{"script":"return document.querySelector(\"#presentation-tier\")?.value ?? \"missing\";","args":[]}' \
    "http://127.0.0.1:$driver_port/session/$session_id/execute/sync" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin).get("value", "error"))')
if [ "$active_tier" != reduced ]; then
    printf '%s\n' "Firefox reduced feasibility client did not report the selected tier" >&2
    exit 1
fi

printf '%s\n' "Firefox smoke test passed: Firefox $firefox_version (normal, default feasibility, and reduced feasibility entry points)"
