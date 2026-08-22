#!/bin/sh
set -eu

app=${FLY_APP_NAME:-pwmtf}
hostname=${PWMTF_HOSTNAME:-pwmtf.hyperchad.dev}
canonical_origin=${PWMTF_CANONICAL_ORIGIN:-https://pwmtf.hyperchad.dev}

require_environment() {
    name=$1
    eval "value=\${$name-}"
    if [ -z "$value" ]; then
        printf '%s\n' "$name is required" >&2
        exit 1
    fi
}

require_environment PWMTF_GOOGLE_CLIENT_ID
require_environment PWMTF_GOOGLE_CLIENT_SECRET

if [ "$hostname" != pwmtf.hyperchad.dev ] || [ "$canonical_origin" != https://pwmtf.hyperchad.dev ]; then
    printf '%s\n' "production deployment must use the canonical PWMTF hostname and origin" >&2
    exit 1
fi

for command in flyctl curl jq; do
    command -v "$command" >/dev/null 2>&1 || {
        printf '%s\n' "$command is required" >&2
        exit 1
    }
done

# Stage both credentials atomically so a partial secret update never launches a
# Machine. Values are read from stdin and are not included in command arguments.
printf 'PWMTF_GOOGLE_CLIENT_ID=%s\nPWMTF_GOOGLE_CLIENT_SECRET=%s\n' \
    "$PWMTF_GOOGLE_CLIENT_ID" "$PWMTF_GOOGLE_CLIENT_SECRET" \
    | flyctl secrets import --app "$app" --stage >/dev/null

flyctl config validate --app "$app"
flyctl deploy --app "$app" --remote-only --ha=false --strategy immediate --wait-timeout 10m

machine_count=$(flyctl status --app "$app" --json \
    | jq '[.Machines[]? | select(.state != "destroyed")] | length')
if [ "$machine_count" -ne 1 ]; then
    printf '%s\n' "production must run exactly one Fly Machine; found $machine_count" >&2
    exit 1
fi

# Fly issues the origin certificate after the proxied AAAA and ACME challenge
# records have propagated. Wait boundedly rather than treating DNS lag as a
# successful deployment.
certificate_ready=false
attempt=0
while [ "$attempt" -lt 60 ]; do
    if [ "$(flyctl certs show "$hostname" --app "$app" --json 2>/dev/null \
        | jq -r '.ClientStatus // empty')" = Ready ]; then
        certificate_ready=true
        break
    fi
    attempt=$((attempt + 1))
    sleep 5
done
if [ "$certificate_ready" != true ]; then
    printf '%s\n' "Fly certificate did not become ready within five minutes" >&2
    exit 1
fi

# Verify the direct health boundary before the broader canonical-origin smoke.
curl --fail --silent --show-error --max-time 20 \
    "https://${hostname}/readyz" | grep -qx ready
./scripts/test-production-smoke.sh
