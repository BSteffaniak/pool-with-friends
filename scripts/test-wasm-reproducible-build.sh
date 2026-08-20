#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

first=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-reproducible-build-a.XXXXXX")
second=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-reproducible-build-b.XXXXXX")
first_optimized=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-reproducible-optimized-a.XXXXXX")
second_optimized=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-reproducible-optimized-b.XXXXXX")
diff_output=$(mktemp "${TMPDIR:-/tmp}/pwmtf-reproducible-build-diff.XXXXXX")
cleanup() {
    rm -rf "$first" "$second" "$first_optimized" "$second_optimized"
    rm -f "$diff_output"
}
trap cleanup EXIT HUP INT TERM

source_hash=$(./scripts/hash-wasm-source.py)
build_id=pwmtf-reproducibility-test-$source_hash
PWMTF_SKIP_WASM_OPT=1 PWMTF_BUILD_ID="$build_id" ./scripts/build-wasm.sh
cp -R dist/. "$first/"
PWMTF_SKIP_WASM_OPT=1 PWMTF_BUILD_ID="$build_id" ./scripts/build-wasm.sh
cp -R dist/. "$second/"

if ! diff -qr "$first" "$second" >"$diff_output"; then
    cat "$diff_output" >&2
    printf '%s\n' "identical PWMTF inputs did not produce an identical generated bundle" >&2
    exit 1
fi
./scripts/verify-wasm-bundle.py "$first"
./scripts/verify-wasm-bundle.py "$second"

PWMTF_BUILD_ID="$build_id" ./scripts/build-wasm.sh
cp -R dist/. "$first_optimized/"
PWMTF_BUILD_ID="$build_id" ./scripts/build-wasm.sh
cp -R dist/. "$second_optimized/"
if ! diff -qr "$first_optimized" "$second_optimized" >"$diff_output"; then
    cat "$diff_output" >&2
    printf '%s\n' "identical PWMTF inputs did not produce an identical wasm-opt bundle" >&2
    exit 1
fi
./scripts/verify-wasm-bundle.py "$first_optimized"
./scripts/verify-wasm-bundle.py "$second_optimized"

grep -q '^const candidateWasmOptimization = "wasm-opt-Oz";$' "$first_optimized/bootstrap.js"

printf '%s\n' "WASM reproducible-build self-test passed"
