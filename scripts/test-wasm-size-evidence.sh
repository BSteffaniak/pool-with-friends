#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-size-evidence-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

if [ ! -f "$root/dist/pwmtf-bundle-manifest.json" ] || \
   ! grep -q '^const candidateWasmOptimization = "wasm-opt-Oz";$' "$root/dist/bootstrap.js"; then
    "$root/scripts/build-wasm.sh"
fi

"$root/scripts/write-wasm-size-evidence.py" \
    --bundle "$root/dist" \
    --output "$tmp/first.json"
"$root/scripts/write-wasm-size-evidence.py" \
    --bundle "$root/dist" \
    --output "$tmp/second.json"
cmp "$tmp/first.json" "$tmp/second.json"

python3 - "$tmp/first.json" "$root/dist/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

evidence = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
manifest = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
assert evidence["schema_version"] == 1
assert evidence["candidate"]["bundle_hash"] == manifest["candidate"]["bundle_hash"]
assert set(evidence["assets"]) == set(manifest["assets"])
assert evidence["totals"]["raw_bytes"] == sum(
    asset["raw_bytes"] for asset in evidence["assets"].values()
)
assert evidence["totals"]["gzip_bytes"] == sum(
    asset["gzip_bytes"] for asset in evidence["assets"].values()
)
PY

PWMTF_SKIP_BUILD=1 "$root/scripts/report-wasm-size.sh" >"$tmp/human.txt"
PWMTF_SIZE_EVIDENCE="$tmp/first.json" PWMTF_HUMAN_SIZE_REPORT="$tmp/human.txt" python3 - <<'PY'
import json
import os
import re
from pathlib import Path

evidence = json.loads(Path(os.environ["PWMTF_SIZE_EVIDENCE"]).read_text(encoding="utf-8"))
report = Path(os.environ["PWMTF_HUMAN_SIZE_REPORT"]).read_text(encoding="utf-8")
match = re.search(r"^TOTAL\s+(\d+)\s+(\d+)", report, re.MULTILINE)
assert match is not None
assert int(match.group(1)) == evidence["totals"]["raw_bytes"]
assert int(match.group(2)) == evidence["totals"]["gzip_bytes"]
assert f"Candidate bundle: {evidence['candidate']['bundle_hash']}" in report
PY

cp -R "$root/dist" "$tmp/tampered"
printf '%s\n' tampered >>"$tmp/tampered/styles.css"
if "$root/scripts/write-wasm-size-evidence.py" \
    --bundle "$tmp/tampered" \
    --output "$tmp/tampered.json" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
    printf '%s\n' "size evidence writer accepted a tampered bundle" >&2
    exit 1
fi
grep -q 'bundle verification failed' "$tmp/rejected.err"
if [ -e "$tmp/tampered.json" ]; then
    printf '%s\n' "failed size evidence generation created an output" >&2
    exit 1
fi

printf '%s\n' "WASM size evidence self-tests passed"
