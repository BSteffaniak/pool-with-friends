#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

package_count=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json, sys; print(len(json.load(sys.stdin)["packages"]))')
if [ "$package_count" -eq 0 ]; then
    printf '%s\n' "native check skipped: the workspace intentionally has no packages yet"
    exit 0
fi

cargo check --workspace --all-targets
