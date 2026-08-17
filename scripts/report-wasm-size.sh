#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

./scripts/build-wasm.sh

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
