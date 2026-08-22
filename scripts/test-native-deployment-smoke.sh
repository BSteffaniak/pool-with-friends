#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ "${PWMTF_WASM_BUNDLE_LOCKED:-0}" != 1 ]; then
    exec ./scripts/with-wasm-bundle-lock.py -- "$0" "$@"
fi

if [ ! -f dist/pwmtf-bundle-manifest.json ]; then
    ./scripts/build-wasm.sh
fi
./scripts/verify-wasm-bundle.py dist

server_bin=${PWMTF_SERVER_BIN:-$root/target/debug/pwmtf-server}
if [ ! -x "$server_bin" ]; then
    cargo build --package pwmtf_server --bin pwmtf-server
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-native-smoke.XXXXXX")
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then
        kill -TERM "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

port=${PWMTF_NATIVE_SMOKE_PORT:-4191}
PWMTF_BIND="127.0.0.1:$port" \
PWMTF_DATABASE_PATH="$tmp/pwmtf.db" \
PWMTF_WEB_ROOT="$root/dist" \
PWMTF_GOOGLE_CLIENT_ID="native-smoke-client" \
PWMTF_GOOGLE_CLIENT_SECRET="native-smoke-secret" \
"$server_bin" >"$tmp/server.log" 2>&1 &
server_pid=$!

attempt=0
while ! curl --fail --silent --output /dev/null "http://127.0.0.1:$port/healthz"; do
    if ! kill -0 "$server_pid" 2>/dev/null; then
        cat "$tmp/server.log" >&2
        printf '%s\n' "native smoke server exited before becoming healthy" >&2
        exit 1
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 200 ]; then
        cat "$tmp/server.log" >&2
        printf '%s\n' "native smoke server did not become healthy" >&2
        exit 1
    fi
    sleep 0.1
done

curl --fail --silent --dump-header "$tmp/health.headers" --output "$tmp/health.body" \
    "http://127.0.0.1:$port/healthz"
[ "$(cat "$tmp/health.body")" = ok ]
grep -qi '^cache-control: no-store' "$tmp/health.headers"
grep -qi '^content-security-policy:' "$tmp/health.headers"
grep -qi '^x-content-type-options: nosniff' "$tmp/health.headers"

curl --fail --silent --dump-header "$tmp/index.headers" --output "$tmp/index.body" \
    "http://127.0.0.1:$port/"
cmp dist/index.html "$tmp/index.body"
grep -qi '^cache-control: no-cache' "$tmp/index.headers"
grep -qi '^content-security-policy:' "$tmp/index.headers"

curl --fail --silent --output "$tmp/spa.body" \
    "http://127.0.0.1:$port/matches/reconnect-example"
cmp dist/index.html "$tmp/spa.body"

for protected_path in api/unknown auth/unknown; do
    curl --silent --dump-header "$tmp/protected.headers" --output "$tmp/protected.body" \
        "http://127.0.0.1:$port/$protected_path"
    [ "$(awk 'NR == 1 { print $2 }' "$tmp/protected.headers")" = 404 ]
    if grep -q 'data-client-state="loading"' "$tmp/protected.body"; then
        printf '%s\n' "native server returned the SPA for protected path $protected_path" >&2
        exit 1
    fi
done

for asset in bootstrap.js brand-mark.svg pwmtf_client_bg.wasm; do
    curl --fail --silent --output "$tmp/$asset" "http://127.0.0.1:$port/$asset"
    cmp "dist/$asset" "$tmp/$asset"
done

curl --silent --dump-header "$tmp/manifest.headers" --output "$tmp/manifest.body" \
    "http://127.0.0.1:$port/pwmtf-bundle-manifest.json"
[ "$(awk 'NR == 1 { print $2 }' "$tmp/manifest.headers")" = 404 ]
if grep -q '"candidate"' "$tmp/manifest.body"; then
    printf '%s\n' "native server exposed the private integrity manifest" >&2
    exit 1
fi

kill -TERM "$server_pid"
wait "$server_pid"
server_pid=

printf '%s\n' "native deployment smoke passed"
