#!/bin/sh
set -eu

app=${FLY_APP_NAME:-pwmtf}
database=${PWMTF_DATABASE_PATH:-/data/pwmtf.db}
backup_directory=${PWMTF_BACKUP_DIRECTORY:-/data/backups}
allow_no_machine=false
if [ "${1:-}" = --if-running ] && [ "$#" -eq 1 ]; then
    allow_no_machine=true
elif [ "$#" -ne 0 ]; then
    printf '%s\n' "usage: $0 [--if-running]" >&2
    exit 2
fi

if [ "$app" != pwmtf ]; then
    printf '%s\n' "production backup must target the canonical pwmtf app" >&2
    exit 1
fi

for command in flyctl jq; do
    command -v "$command" >/dev/null 2>&1 || {
        printf '%s\n' "$command is required" >&2
        exit 1
    }
done

case "$database" in
    /data/*.db) ;;
    *)
        printf '%s\n' "PWMTF_DATABASE_PATH must name a database directly under /data" >&2
        exit 1
        ;;
esac
case "$backup_directory" in
    /data/*) ;;
    *)
        printf '%s\n' "PWMTF_BACKUP_DIRECTORY must be under /data" >&2
        exit 1
        ;;
esac

status=$(flyctl status --app "$app" --json)
started_machine_count=$(jq '[.Machines[]? | select(.state == "started")] | length' <<EOF
$status
EOF
)
if [ "$allow_no_machine" = true ] && [ "$started_machine_count" -eq 0 ]; then
    printf '%s\n' "No started production Machine; backup skipped"
    exit 0
fi
machine_id=$(jq -er '[.Machines[]? | select(.state == "started")] | if length == 1 then .[0].id else error("expected exactly one started Machine") end' <<EOF
$status
EOF
)
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup="$backup_directory/pwmtf-$timestamp.db"
restore="$backup_directory/.restore-check-$timestamp.db"

# Arguments are generated from fixed prefixes and a UTC timestamp. The runtime
# image owns the same reviewed backup/restore scripts used by local validation.
remote_command="set -eu; mkdir -p '$backup_directory'; /app/scripts/backup-database.sh '$database' '$backup' >/dev/null; /app/scripts/restore-database.sh '$backup' '$restore' >/dev/null; sqlite3 '$restore' 'PRAGMA quick_check' | grep -qx ok; rm -f '$restore'; sync '$backup_directory'; printf '%s\\n' '$backup'"
# Fly exec does not implicitly invoke a shell. Quote the whole program as one
# POSIX shell argument, including embedded single quotes in generated paths.
quoted_command=$(printf '%s' "$remote_command" | sed "s/'/'\\\\''/g")
result=$(flyctl machine exec --app "$app" "$machine_id" "/bin/sh -c '$quoted_command'" --timeout 120 --json)
# Fly omits exit_code on success, but also on some exec failures. Require the
# expected success output and empty stderr, not just an absent exit code.
if ! printf '%s' "$result" | jq -e --arg expected "$backup" '
    type == "object" and
    (if has("exit_code") then .exit_code == 0 else true end) and
    ((.stderr // "") == "") and (.stdout == ($expected + "\n"))
' >/dev/null; then
    printf '%s\n' "remote backup execution failed or returned unexpected output; response suppressed" >&2
    exit 1
fi
printf '%s\n' "$backup"
