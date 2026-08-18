#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

source_hash=$(python3 - <<'PY'
from __future__ import annotations

import hashlib
from pathlib import Path

root = Path.cwd()
inputs = [root / "Cargo.lock", root / "Cargo.toml", root / "rust-toolchain.toml"]
inputs.extend(path for directory in (root / ".cargo", root / "packages" / "client") for path in directory.rglob("*") if path.is_file())
digest = hashlib.sha256()
for path in sorted(inputs, key=lambda candidate: candidate.relative_to(root).as_posix()):
    relative = path.relative_to(root).as_posix().encode()
    contents = path.read_bytes()
    digest.update(len(relative).to_bytes(4, "big"))
    digest.update(relative)
    digest.update(len(contents).to_bytes(8, "big"))
    digest.update(contents)
print(digest.hexdigest())
PY
)

build_id=${PWMTF_BUILD_ID:-}
if [ -z "$build_id" ]; then
    revision=$(git rev-parse --short=12 HEAD 2>/dev/null || printf '%s' unknown)
    build_id="$revision-$source_hash"
fi
case "$build_id" in
    *[!A-Za-z0-9._-]*|'')
        printf '%s\n' "PWMTF_BUILD_ID must contain only ASCII letters, digits, dot, underscore, or hyphen" >&2
        exit 1
        ;;
esac

cargo build --package pwmtf_client --target wasm32-unknown-unknown --release
mkdir -p dist
find dist -mindepth 1 -maxdepth 1 -type f -delete
cp packages/client/web/index.html packages/client/web/styles.css packages/client/web/bootstrap.js dist/
wasm_optimization=not-applied
if command -v wasm-opt >/dev/null 2>&1 && [ "${PWMTF_SKIP_WASM_OPT:-0}" != 1 ]; then
    wasm_optimization=wasm-opt-Oz
fi
BUILD_ID="$build_id" SOURCE_HASH="$source_hash" WASM_OPTIMIZATION="$wasm_optimization" python3 - <<'PY'
import os
from pathlib import Path

path = Path("dist/bootstrap.js")
contents = path.read_text(encoding="utf-8")
replacements = {
    "__PWMTF_BUILD_ID__": os.environ["BUILD_ID"],
    "__PWMTF_SOURCE_HASH__": os.environ["SOURCE_HASH"],
    "__PWMTF_WASM_OPTIMIZATION__": os.environ["WASM_OPTIMIZATION"],
}
for placeholder, value in replacements.items():
    if contents.count(placeholder) != 1:
        raise SystemExit(f"bootstrap placeholder must occur exactly once: {placeholder}")
    contents = contents.replace(placeholder, value)
path.write_text(contents, encoding="utf-8")
PY
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

printf '%s\n' "PWMTF web client built in $root/dist (build $build_id)"
