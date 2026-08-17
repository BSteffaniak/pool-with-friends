#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

cargo build --package pwmtf_client --target wasm32-unknown-unknown --release
mkdir -p dist
find dist -mindepth 1 -maxdepth 1 -type f -delete
cp packages/client/web/index.html packages/client/web/styles.css packages/client/web/bootstrap.js dist/
wasm_schema=$(grep -m1 'name = "wasm-bindgen"' -A2 Cargo.lock | grep 'version = ' | cut -d'"' -f2)
cli_schema=$(wasm-bindgen --version | awk '{print $2}')
if [ "$wasm_schema" != "$cli_schema" ]; then
    printf '%s\n' "wasm-bindgen CLI $cli_schema does not match locked crate $wasm_schema" >&2
    printf '%s\n' "install wasm-bindgen-cli $wasm_schema, then rerun this script" >&2
    exit 1
fi
wasm-bindgen \
    --target web \
    --out-dir dist \
    --out-name pwmtf_client \
    target/wasm32-unknown-unknown/release/pwmtf-client.wasm

if command -v wasm-opt >/dev/null 2>&1; then
    optimized="$root/dist/pwmtf_client_bg.optimized.wasm"
    if [ "${PWMTF_SKIP_WASM_OPT:-0}" = 1 ]; then
        printf '%s\n' "PWMTF_SKIP_WASM_OPT=1: leaving release WASM unoptimized by Binaryen" >&2
    else
        wasm-opt \
            -Oz \
            --enable-bulk-memory \
            --enable-multivalue \
            --enable-mutable-globals \
            --enable-nontrapping-float-to-int \
            --enable-reference-types \
            --enable-sign-ext \
            "$root/dist/pwmtf_client_bg.wasm" \
            -o "$optimized"
        mv "$optimized" "$root/dist/pwmtf_client_bg.wasm"
    fi
fi

printf '%s\n' "PWMTF web client built in $root/dist"
