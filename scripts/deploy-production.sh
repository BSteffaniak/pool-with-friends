#!/bin/sh
set -eu

app=${FLY_APP_NAME:-pwmtf}
hostname=${PWMTF_HOSTNAME:-pwmtf.hyperchad.dev}
canonical_origin=${PWMTF_CANONICAL_ORIGIN:-https://pwmtf.hyperchad.dev}
volume_name=${FLY_VOLUME_NAME:-pwmtf_data}
production_smoke=${PWMTF_PRODUCTION_SMOKE_SCRIPT:-./scripts/test-production-smoke.sh}
production_smoke_argument=${PWMTF_PRODUCTION_SMOKE_ARGUMENT:---origin-only}

require_environment() {
    name=$1
    eval "value=\${$name-}"
    if [ -z "$value" ]; then
        printf '%s\n' "$name is required" >&2
        exit 1
    fi
}

require_environment PWMTF_DEPLOY_IMAGE
if ! printf '%s\n' "$PWMTF_DEPLOY_IMAGE" | grep -Eq '^registry\.fly\.io/pwmtf:build-[0-9a-f]{40}-[0-9]+-[0-9]+$'; then
    printf '%s\n' "PWMTF_DEPLOY_IMAGE must be a pwmtf build tag containing commit SHA, run ID, and attempt" >&2
    exit 1
fi
require_environment PWMTF_GOOGLE_CLIENT_ID
require_environment PWMTF_GOOGLE_CLIENT_SECRET

if [ "$app" != pwmtf ] || [ "$hostname" != pwmtf.hyperchad.dev ] || [ "$canonical_origin" != https://pwmtf.hyperchad.dev ]; then
    printf '%s\n' "production deployment must use the canonical PWMTF app, hostname, and origin" >&2
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
volumes=$(flyctl volumes list --app "$app" --json)
volume_count=$(jq --arg name "$volume_name" '[.[] | select((.name // .Name) == $name and (.state // .State) == "created" and (.encrypted // .Encrypted) == true and (.region // .Region) == "ord" and (.snapshot_retention // .SnapshotRetention) == 14 and (.auto_backup_enabled // .AutoBackupEnabled) == true)] | length' <<EOF
$volumes
EOF
)
if [ "$volume_count" -ne 1 ]; then
    printf '%s\n' "production requires exactly one created encrypted $volume_name volume in ord with 14-day snapshots and automatic backups; found $volume_count" >&2
    exit 1
fi
flyctl deploy --app "$app" --image "$PWMTF_DEPLOY_IMAGE" --ha=false --strategy immediate --wait-timeout 10m

wait_for_started_machine() {
    attempt=0
    while [ "$attempt" -lt 60 ]; do
        snapshot=$(flyctl status --app "$app" --json) || return 1
        count=$(printf '%s' "$snapshot" | jq '[.Machines[]? | select(.state != "destroyed")] | length')
        if [ "$count" -gt 1 ]; then
            printf 'production must run exactly one Fly Machine; found %s\n' "$count" >&2
            return 1
        fi
        started=$(printf '%s' "$snapshot" | jq -r '.Machines[]? | select(.state == "started") | .id')
        if [ "$count" -eq 1 ] && [ -n "$started" ]; then
            printf '%s\n' "$started"
            return 0
        fi
        printf 'Waiting for production Machine (%s/60); states: ' "$((attempt + 1))" >&2
        printf '%s' "$snapshot" | jq -c '[.Machines[]? | {id,state}]' >&2
        attempt=$((attempt + 1))
        sleep 5
    done
    printf '%s\n' 'Production Machine did not start within five minutes; inspect Fly Machine events' >&2
    return 1
}
machine_id=$(wait_for_started_machine)
machines=$(flyctl machine list --app "$app" --json)
machine_configuration=$(printf '%s' "$machines" | jq -er --arg id "$machine_id" '
    [.[] | select(.id == $id)] |
    if length == 1 and (.[0].config | type) == "object" then .[0].config
    else error("expected exactly one matching Machine with configuration") end')
if ! jq -e --arg volume "$volume_name" '
    any(.mounts[]?; .name == $volume and .path == "/data" and .encrypted == true)
    and any(.services[]?;
        .internal_port == 8080
        and .autostop == false
        and .autostart == true
        and .min_machines_running == 1
        and any(.ports[]?; .port == 80 and .force_https == true and (.handlers | index("http") != null))
        and any(.ports[]?; .port == 443 and (.handlers | index("http") != null) and (.handlers | index("tls") != null))
        and any(.checks[]?; .type == "http" and .path == "/readyz"))
' <<EOF
$machine_configuration
EOF
then
    printf '%s\n' "production Machine does not preserve the required volume, availability, and readiness configuration" >&2
    exit 1
fi
identity_command='set -eu; build=$(cat /app/pwmtf-build-id); source=$(cat /app/pwmtf-source-hash); test -n "$build"; test ${#source} -eq 64; grep -Fq "const candidateBuildId = \"$build\";" /app/dist/bootstrap.js; grep -Fq "const candidateSourceHash = \"$source\";" /app/dist/bootstrap.js'
identity_command="$identity_command; printf '%s\\n' pwmtf-identity-ok"
quoted_command=$(printf '%s' "$identity_command" | sed "s/'/'\\\\''/g")
identity_result=$(flyctl machine exec --app "$app" "$machine_id" "/bin/sh -c '$quoted_command'" --timeout 30 --json)
if ! jq -e 'type == "object" and (if has("exit_code") then .exit_code == 0 else true end) and ((.stderr // "") == "") and .stdout == "pwmtf-identity-ok\n"' >/dev/null <<EOF
$identity_result
EOF
then
    printf '%s\n' "production Machine browser bundle identity does not match its immutable image identity" >&2
    exit 1
fi

# Probe the actual HTTPS boundary rather than parsing version-specific Fly
# certificate status fields. DNS/TLS provisioning is a separate approved workflow.
certificate_ready=false
attempt=0
while [ "$attempt" -lt 30 ]; do
    if curl --fail --silent --show-error --connect-timeout 5 --max-time 10 \
        "https://${hostname}/readyz" | grep -qx ready; then
        certificate_ready=true
        break
    fi
    attempt=$((attempt + 1))
    sleep 5
done
if [ "$certificate_ready" != true ]; then
    printf '%s\n' "Canonical HTTPS readiness failed after 30 attempts; check the infrastructure workflow, DNS, TLS, and Machine logs" >&2
    exit 1
fi

# Verify the direct health boundary before the broader canonical-origin smoke.
curl --fail --silent --show-error --max-time 20 \
    "https://${hostname}/readyz" | grep -qx ready
"$production_smoke" "$production_smoke_argument"

# Qualify the real process-recovery boundary on every deployment. This proves
# startup migrations and recovery for the deployed database head; acceptance
# after real play remains a separate product criterion.
flyctl machine restart "$machine_id" --app "$app" --signal SIGTERM --time 30
restarted_machine_id=$(wait_for_started_machine)
if [ "$restarted_machine_id" != "$machine_id" ]; then
    printf '%s\n' "production restart changed the canonical Machine identity" >&2
    exit 1
fi
curl --fail --silent --show-error --max-time 20 \
    "https://${hostname}/readyz" | grep -qx ready
"$production_smoke" "$production_smoke_argument"
