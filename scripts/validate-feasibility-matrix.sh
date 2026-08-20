#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if [ "${PWMTF_WASM_BUNDLE_LOCKED:-0}" != 1 ]; then
    exec "$root/scripts/with-wasm-bundle-lock.py" -- "$0" "$@"
fi

reports=${PWMTF_FEASIBILITY_REPORTS:-$root/feasibility-reports}
bundle=${PWMTF_FEASIBILITY_BUNDLE:-$root/dist}
decision=${PWMTF_FEASIBILITY_DECISION:-$reports/acceptance-decision.md}
temporary_decision=$decision.tmp.$$
size_evidence=${PWMTF_FEASIBILITY_SIZE_EVIDENCE:-$reports/wasm-size-evidence.json}
temporary_size_evidence=$size_evidence.tmp.$$
cleanup() {
    rm -f "$temporary_decision" "$temporary_size_evidence"
}
trap cleanup EXIT HUP INT TERM

if [ "$#" -gt 0 ]; then
    printf '%s\n' "usage: PWMTF_FEASIBILITY_REPORTS=path PWMTF_FEASIBILITY_BUNDLE=path PWMTF_FEASIBILITY_DECISION=path PWMTF_FEASIBILITY_SIZE_EVIDENCE=path $0" >&2
    exit 2
fi
if [ ! -d "$reports" ]; then
    printf '%s\n' "physical feasibility report directory does not exist: $reports" >&2
    exit 1
fi
if [ ! -d "$bundle" ]; then
    printf '%s\n' "expected WASM bundle directory does not exist: $bundle" >&2
    exit 1
fi
if ! "$root/scripts/verify-wasm-bundle.py" "$bundle"; then
    printf '%s\n' "final feasibility validation requires an untampered verified WASM bundle" >&2
    exit 1
fi

decision_directory=$(dirname -- "$decision")
size_evidence_directory=$(dirname -- "$size_evidence")
if [ ! -d "$decision_directory" ]; then
    printf '%s\n' "acceptance decision directory does not exist: $decision_directory" >&2
    exit 1
fi
if [ ! -d "$size_evidence_directory" ]; then
    printf '%s\n' "size evidence directory does not exist: $size_evidence_directory" >&2
    exit 1
fi

if [ -e "$decision" ]; then
    printf '%s\n' "acceptance decision already exists; remove it explicitly before validating a replacement: $decision" >&2
    exit 1
fi
if [ -e "$size_evidence" ]; then
    printf '%s\n' "size evidence already exists; remove it explicitly before validating a replacement: $size_evidence" >&2
    exit 1
fi

set -- "$reports"/*.json
if [ ! -e "$1" ]; then
    printf '%s\n' "physical feasibility report directory contains no JSON reports: $reports" >&2
    exit 1
fi

"$root/scripts/summarize-feasibility.py" \
    --require-mobile-matrix \
    --expected-bundle "$bundle" \
    --write-decision "$temporary_decision" \
    "$@"
"$root/scripts/write-wasm-size-evidence.py" \
    --bundle "$bundle" \
    --output "$temporary_size_evidence"
mv "$temporary_size_evidence" "$size_evidence"
mv "$temporary_decision" "$decision"
