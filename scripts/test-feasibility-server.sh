#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-feasibility-server-test.XXXXXX")
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

for tool in curl openssl python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf '%s\n' "feasibility server self-test requires $tool" >&2
        exit 1
    fi
done

if [ ! -f "$root/dist/bootstrap.js" ]; then
    printf '%s\n' "feasibility server self-test requires an existing optimized dist bundle" >&2
    exit 1
fi

openssl req \
    -x509 \
    -newkey rsa:2048 \
    -keyout "$tmp/key.pem" \
    -out "$tmp/cert.pem" \
    -days 2 \
    -nodes \
    -subj '/CN=localhost' \
    -addext 'subjectAltName=DNS:localhost' \
    >"$tmp/openssl.log" 2>&1
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$tmp/wrong-key.pem" \
    >>"$tmp/openssl.log" 2>&1

expect_rejected() {
    label=$1
    shift
    if "$@" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
        printf '%s\n' "feasibility server accepted $label" >&2
        exit 1
    fi
}

expect_rejected \
    "a certificate for another host" \
    env \
    PWMTF_SKIP_BUILD=1 \
    PWMTF_FEASIBILITY_HOST=wrong.example \
    PWMTF_TLS_CERT="$tmp/cert.pem" \
    PWMTF_TLS_KEY="$tmp/key.pem" \
    "$root/scripts/serve-feasibility.sh"
grep -q 'certificate does not cover' "$tmp/rejected.err"

expect_rejected \
    "a mismatched private key" \
    env \
    PWMTF_SKIP_BUILD=1 \
    PWMTF_FEASIBILITY_HOST=localhost \
    PWMTF_TLS_CERT="$tmp/cert.pem" \
    PWMTF_TLS_KEY="$tmp/wrong-key.pem" \
    "$root/scripts/serve-feasibility.sh"
grep -q 'private key does not match' "$tmp/rejected.err"

port=${PWMTF_FEASIBILITY_TEST_PORT:-48444}
env \
    PWMTF_SKIP_BUILD=1 \
    PWMTF_FEASIBILITY_HOST=localhost \
    PWMTF_TLS_CERT="$tmp/cert.pem" \
    PWMTF_TLS_KEY="$tmp/key.pem" \
    PWMTF_FEASIBILITY_BIND=127.0.0.1 \
    PWMTF_FEASIBILITY_PORT="$port" \
    "$root/scripts/serve-feasibility.sh" >"$tmp/server.log" 2>&1 &
server_pid=$!

attempt=0
while ! curl --insecure --fail --silent --output /dev/null "https://127.0.0.1:$port/"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        cat "$tmp/server.log" >&2
        printf '%s\n' "local feasibility HTTPS server did not start" >&2
        exit 1
    fi
    sleep 0.1
done

curl \
    --insecure \
    --fail \
    --silent \
    --dump-header "$tmp/headers" \
    --output /dev/null \
    "https://127.0.0.1:$port/?feasibility"
grep -qi '^Cache-Control: no-store' "$tmp/headers"
grep -qi '^Permissions-Policy: camera=(), geolocation=(), microphone=()' "$tmp/headers"
grep -qi '^Referrer-Policy: no-referrer' "$tmp/headers"
grep -qi '^X-Content-Type-Options: nosniff' "$tmp/headers"

printf '%s\n' "feasibility HTTPS server self-tests passed"
