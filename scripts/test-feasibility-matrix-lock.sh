#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-matrix-lock-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

mkdir -p "$tmp/reports" "$tmp/evidence"
printf '%s\n' '{}' >"$tmp/reports/invalid.json"

PWMTF_WASM_BUNDLE_LOCKED=1 "$root/scripts/build-wasm.sh"

PWMTF_WASM_BUNDLE_LOCKED=1 python3 - "$root" "$tmp" <<'PY' &
import fcntl
import subprocess
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
tmp = Path(sys.argv[2])
lock_path = root / "target" / "pwmtf-wasm-bundle.lock"
with lock_path.open("a+b") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    marker = tmp / "holder-ready"
    marker.write_text("ready\n", encoding="utf-8")
    time.sleep(1)
    subprocess.run([str(root / "scripts" / "build-wasm.sh")], cwd=root, check=True)
PY
holder_pid=$!

attempt=0
while [ ! -f "$tmp/holder-ready" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ]; then
        printf '%s\n' "bundle lock holder did not start" >&2
        exit 1
    fi
    sleep 0.02
done

if PWMTF_FEASIBILITY_REPORTS="$tmp/reports" \
    PWMTF_FEASIBILITY_DECISION="$tmp/evidence/decision.md" \
    PWMTF_FEASIBILITY_SIZE_EVIDENCE="$tmp/evidence/size.json" \
    "$root/scripts/validate-feasibility-matrix.sh" >"$tmp/validation.out" 2>"$tmp/validation.err"; then
    printf '%s\n' "matrix validator accepted an invalid report" >&2
    exit 1
fi
wait "$holder_pid"

if ! grep -q 'report root keys are invalid' "$tmp/validation.err"; then
    cat "$tmp/validation.err" >&2
    printf '%s\n' "matrix validator did not report the expected invalid fixture" >&2
    exit 1
fi
if grep -q 'WASM bundle integrity error' "$tmp/validation.err"; then
    cat "$tmp/validation.err" >&2
    printf '%s\n' "matrix validation observed a changing generated bundle" >&2
    exit 1
fi
if [ -e "$tmp/evidence/decision.md" ] || [ -e "$tmp/evidence/size.json" ]; then
    printf '%s\n' "failed matrix validation wrote final evidence" >&2
    exit 1
fi

printf '%s\n' "feasibility matrix bundle-lock self-test passed"
