#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ "${PWMTF_WASM_BUNDLE_LOCKED:-0}" != 1 ]; then
    exec ./scripts/with-wasm-bundle-lock.py -- "$0" "$@"
fi

public_host=${PWMTF_FEASIBILITY_HOST:-}
certificate=${PWMTF_TLS_CERT:-}
private_key=${PWMTF_TLS_KEY:-}
bind_address=${PWMTF_FEASIBILITY_BIND:-0.0.0.0}
port=${PWMTF_FEASIBILITY_PORT:-8443}

for tool in openssl python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf '%s\n' "physical feasibility serving requires $tool" >&2
        exit 1
    fi
done
if [ -z "$public_host" ]; then
    printf '%s\n' "PWMTF_FEASIBILITY_HOST must name the host or IP covered by the TLS certificate" >&2
    exit 1
fi
case "$port" in
    ''|*[!0-9]*)
        printf '%s\n' "PWMTF_FEASIBILITY_PORT must be an integer from 1 through 65535" >&2
        exit 1
        ;;
esac
if [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
    printf '%s\n' "PWMTF_FEASIBILITY_PORT must be an integer from 1 through 65535" >&2
    exit 1
fi
if [ -z "$certificate" ] || [ -z "$private_key" ]; then
    printf '%s\n' "PWMTF_TLS_CERT and PWMTF_TLS_KEY must point to a device-trusted TLS certificate and private key" >&2
    exit 1
fi
if [ ! -r "$certificate" ]; then
    printf '%s\n' "TLS certificate is not readable: $certificate" >&2
    exit 1
fi
if [ ! -r "$private_key" ]; then
    printf '%s\n' "TLS private key is not readable: $private_key" >&2
    exit 1
fi
if [ "$certificate" = "$private_key" ]; then
    printf '%s\n' "TLS certificate and private key must be separate files" >&2
    exit 1
fi
if ! openssl x509 -in "$certificate" -noout -checkend 86400 >/dev/null 2>&1; then
    printf '%s\n' "TLS certificate is invalid, expired, or expires within 24 hours" >&2
    exit 1
fi
certificate_public_key=$(openssl x509 -in "$certificate" -pubkey -noout 2>/dev/null | openssl pkey -pubin -outform DER 2>/dev/null | openssl dgst -sha256)
private_public_key=$(openssl pkey -in "$private_key" -pubout -outform DER 2>/dev/null | openssl dgst -sha256)
if [ -z "$certificate_public_key" ] || [ "$certificate_public_key" != "$private_public_key" ]; then
    printf '%s\n' "TLS private key does not match the certificate" >&2
    exit 1
fi

if ! openssl x509 -in "$certificate" -noout -checkhost "$public_host" >/dev/null 2>&1 && \
   ! openssl x509 -in "$certificate" -noout -checkip "$public_host" >/dev/null 2>&1; then
    printf '%s\n' "TLS certificate does not cover PWMTF_FEASIBILITY_HOST: $public_host" >&2
    exit 1
fi

if [ "${PWMTF_SKIP_BUILD:-0}" != 1 ]; then
    ./scripts/build-wasm.sh
elif [ ! -f dist/index.html ] || [ ! -f dist/bootstrap.js ]; then
    printf '%s\n' "PWMTF_SKIP_BUILD=1 requires an existing identified dist bundle" >&2
    exit 1
fi

if ! ./scripts/verify-wasm-bundle.py dist; then
    printf '%s\n' "physical feasibility serving requires a complete untampered WASM bundle" >&2
    exit 1
fi

candidate_identity=$(python3 - <<'PY'
import re
from pathlib import Path

contents = Path("dist/bootstrap.js").read_text(encoding="utf-8")
build = re.search(r'^const candidateBuildId = "([A-Za-z0-9._-]+)";$', contents, re.MULTILINE)
source = re.search(r'^const candidateSourceHash = "([0-9a-f]{64})";$', contents, re.MULTILINE)
optimization = re.search(
    r'^const candidateWasmOptimization = "(wasm-opt-Oz|not-applied)";$', contents, re.MULTILINE
)
if build is None or source is None or optimization is None:
    raise SystemExit("dist bundle does not contain a valid candidate identity")
if source.group(1) not in build.group(1):
    raise SystemExit("candidate build ID does not include candidate source hash")
if optimization.group(1) != "wasm-opt-Oz":
    raise SystemExit("physical feasibility serving requires a wasm-opt-Oz candidate")
print(f"build {build.group(1)} / source {source.group(1)} / {optimization.group(1)}")
PY
)

printf '%s\n' "Serving PWMTF candidate $candidate_identity"
printf '%s\n' "Keep this terminal open while testing; press Ctrl-C to stop."

exec ./scripts/serve-wasm-bundle.py \
    --bind "$bind_address" \
    --port "$port" \
    --directory "$root/dist" \
    --certificate "$certificate" \
    --private-key "$private_key"
