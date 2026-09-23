#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-bundle-lock-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

run_test() {
    name=$1
    shift
    printf '%s\n' "START $name $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    if "$@" >"$tmp/$name.log" 2>&1; then
        printf '%s\n' 0 >"$tmp/$name.status"
    else
        printf '%s\n' "$?" >"$tmp/$name.status"
    fi
    printf '%s\n' "END $name exit=$(cat "$tmp/$name.status") $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
}

run_test build "$root/scripts/build-wasm.sh" &
build_pid=$!
run_test chrome "$root/scripts/test-browser-smoke.sh" &
chrome_pid=$!
run_test edge "$root/scripts/test-edge-smoke.sh" &
edge_pid=$!
run_test tools "$root/scripts/test-feasibility-tools.sh" &
tools_pid=$!
run_test integrity "$root/scripts/test-wasm-bundle-integrity.sh" &
integrity_pid=$!

wait "$build_pid"
wait "$chrome_pid"
wait "$edge_pid"
wait "$tools_pid"
wait "$integrity_pid"

if [ -n "${PWMTF_TEST_ARTIFACT_DIR:-}" ]; then
    mkdir -p "$PWMTF_TEST_ARTIFACT_DIR"
    cp "$tmp/"*.log "$tmp/"*.status "$PWMTF_TEST_ARTIFACT_DIR/"
fi

failed=0
for name in build chrome edge tools integrity; do
    status=$(cat "$tmp/$name.status")
    if [ "$status" != 0 ]; then
        cat "$tmp/$name.log" >&2
        printf '%s\n' "concurrent WASM bundle operation failed: $name" >&2
        failed=1
    fi
done
if [ "$failed" != 0 ]; then
    exit 1
fi

printf '%s\n' "WASM bundle concurrency self-test passed"
