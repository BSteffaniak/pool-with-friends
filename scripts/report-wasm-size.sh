#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
tmp=$(mktemp "${TMPDIR:-/tmp}/pwmtf-wasm-size-evidence.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM

if [ "${PWMTF_SKIP_BUILD:-0}" != 1 ]; then
    ./scripts/build-wasm.sh
elif [ ! -f dist/pwmtf-bundle-manifest.json ]; then
    printf '%s\n' "PWMTF_SKIP_BUILD=1 requires an existing verified dist bundle" >&2
    exit 1
fi

./scripts/write-wasm-size-evidence.py --bundle dist --output "$tmp"
PWMTF_SIZE_EVIDENCE="$tmp" \
PWMTF_ALLOW_UNOPTIMIZED_SIZE_REPORT="${PWMTF_ALLOW_UNOPTIMIZED_SIZE_REPORT:-0}" \
python3 - <<'PY'
from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Any

root = Path.cwd()
evidence: dict[str, Any] = json.loads(
    Path(os.environ["PWMTF_SIZE_EVIDENCE"]).read_text(encoding="utf-8")
)
candidate = evidence["candidate"]
if (
    candidate["wasm_optimization"] != "wasm-opt-Oz"
    and os.environ["PWMTF_ALLOW_UNOPTIMIZED_SIZE_REPORT"] != "1"
):
    raise SystemExit(
        "size evidence requires a wasm-opt-Oz candidate; "
        "set PWMTF_ALLOW_UNOPTIMIZED_SIZE_REPORT=1 only for troubleshooting"
    )

print(f"Candidate build: {candidate['build_id']}")
print(f"Candidate source: {candidate['source_hash']}")
print(f"Candidate bundle: {candidate['bundle_hash']}")
print(f"Candidate bundle algorithm: {candidate['bundle_hash_algorithm']}")
print(f"Candidate WASM optimization: {candidate['wasm_optimization']}")

brotli = shutil.which("brotli")
header = f"{'Asset':<36} {'Raw bytes':>12} {'Gzip bytes':>12}"
if brotli is not None:
    header += f" {'Brotli bytes':>12}"
print(header)

brotli_total = 0
for name, metadata in evidence["assets"].items():
    line = f"{name:<36} {metadata['raw_bytes']:>12} {metadata['gzip_bytes']:>12}"
    if brotli is not None:
        result = subprocess.run(
            [brotli, "--quality=11", "--stdout", root / "dist" / name],
            check=True,
            stdout=subprocess.PIPE,
        )
        brotli_bytes = len(result.stdout)
        brotli_total += brotli_bytes
        line += f" {brotli_bytes:>12}"
    print(line)

totals = evidence["totals"]
line = f"{'TOTAL':<36} {totals['raw_bytes']:>12} {totals['gzip_bytes']:>12}"
if brotli is not None:
    line += f" {brotli_total:>12}"
print(line)
PY
