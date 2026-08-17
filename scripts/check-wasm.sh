#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if ! cargo metadata --format-version=1 --no-deps | python3 -c 'import json, sys; raise SystemExit(not any(package["name"] == "pwmtf_client" for package in json.load(sys.stdin)["packages"]))'; then
    printf '%s\n' "WASM check skipped: pwmtf_client has not been introduced yet"
    exit 0
fi

cargo check --package pwmtf_client --target wasm32-unknown-unknown
