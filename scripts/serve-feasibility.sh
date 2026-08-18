#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

public_host=${PWMTF_FEASIBILITY_HOST:-}
certificate=${PWMTF_TLS_CERT:-}
private_key=${PWMTF_TLS_KEY:-}
bind_address=${PWMTF_FEASIBILITY_BIND:-0.0.0.0}
port=${PWMTF_FEASIBILITY_PORT:-8443}

if [ -z "$public_host" ]; then
    printf '%s\n' "PWMTF_FEASIBILITY_HOST must name the host or IP covered by the TLS certificate" >&2
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

if [ "${PWMTF_SKIP_BUILD:-0}" != 1 ]; then
    ./scripts/build-wasm.sh
elif [ ! -f dist/index.html ] || [ ! -f dist/bootstrap.js ]; then
    printf '%s\n' "PWMTF_SKIP_BUILD=1 requires an existing identified dist bundle" >&2
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

exec python3 - "$bind_address" "$port" "$root/dist" "$certificate" "$private_key" <<'PY'
from __future__ import annotations

import http.server
import os
import ssl
import sys

bind_address, port, directory, certificate, private_key = sys.argv[1:]


class FeasibilityHandler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self) -> None:
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Permissions-Policy", "camera=(), geolocation=(), microphone=()")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


handler = lambda *args, **kwargs: FeasibilityHandler(  # noqa: E731
    *args, directory=os.path.realpath(directory), **kwargs
)
server = http.server.ThreadingHTTPServer((bind_address, int(port)), handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(certificate, private_key)
server.socket = context.wrap_socket(server.socket, server_side=True)
try:
    server.serve_forever()
except KeyboardInterrupt:
    pass
finally:
    server.server_close()
PY
