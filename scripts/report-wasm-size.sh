#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ "${PWMTF_SKIP_BUILD:-0}" != 1 ]; then
    ./scripts/build-wasm.sh
elif [ ! -f dist/bootstrap.js ]; then
    printf '%s\n' "PWMTF_SKIP_BUILD=1 requires an existing dist/bootstrap.js" >&2
    exit 1
fi

build_id=$(python3 - <<'PY'
import re
from pathlib import Path

contents = Path("dist/bootstrap.js").read_text(encoding="utf-8")
match = re.search(r'^const candidateBuildId = "([A-Za-z0-9._-]+)";$', contents, re.MULTILINE)
if match is None:
    raise SystemExit("cannot read candidate build ID from dist/bootstrap.js")
print(match.group(1))
PY
)
source_hash=$(python3 - <<'PY'
import re
from pathlib import Path

contents = Path("dist/bootstrap.js").read_text(encoding="utf-8")
match = re.search(r'^const candidateSourceHash = "([0-9a-f]{64})";$', contents, re.MULTILINE)
if match is None:
    raise SystemExit("cannot read candidate source hash from dist/bootstrap.js")
print(match.group(1))
PY
)
printf '%s\n' "Candidate build: $build_id"
printf '%s\n' "Candidate source: $source_hash"

printf '%-36s %12s %12s' 'Asset' 'Raw bytes' 'Gzip bytes'
if command -v brotli >/dev/null 2>&1; then
    printf ' %12s' 'Brotli bytes'
fi
printf '\n'

raw_total=0
gzip_total=0
brotli_total=0
for asset in dist/*; do
    if [ ! -f "$asset" ]; then
        continue
    fi

    raw_bytes=$(wc -c < "$asset" | tr -d '[:space:]')
    gzip_bytes=$(gzip -9 -c "$asset" | wc -c | tr -d '[:space:]')
    raw_total=$((raw_total + raw_bytes))
    gzip_total=$((gzip_total + gzip_bytes))

    printf '%-36s %12s %12s' "${asset#dist/}" "$raw_bytes" "$gzip_bytes"
    if command -v brotli >/dev/null 2>&1; then
        brotli_bytes=$(brotli --quality=11 --stdout "$asset" | wc -c | tr -d '[:space:]')
        brotli_total=$((brotli_total + brotli_bytes))
        printf ' %12s' "$brotli_bytes"
    fi
    printf '\n'
done

printf '%-36s %12s %12s' 'TOTAL' "$raw_total" "$gzip_total"
if command -v brotli >/dev/null 2>&1; then
    printf ' %12s' "$brotli_total"
fi
printf '\n'
