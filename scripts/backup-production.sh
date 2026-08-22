#!/bin/sh
set -eu

app=${FLY_APP_NAME:-pwmtf}
database=${PWMTF_DATABASE_PATH:-/data/pwmtf.db}
backup_directory=${PWMTF_BACKUP_DIRECTORY:-/data/backups}

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
result=$(flyctl machine exec --app "$app" "$machine_id" "$remote_command" --timeout 120 --json)
jq -e '(.exit_code // 0) == 0' <<EOF
$result
EOF
 >/dev/null
jq -rj '.stderr // empty' <<EOF
$result
EOF
 >&2
created=$(jq -rj '.stdout // empty' <<EOF
$result
EOF
)
if [ "$created" != "$backup" ]; then
    printf '%s\n' "remote backup did not report the expected path" >&2
    exit 1
fi
printf '%s\n' "$backup"
