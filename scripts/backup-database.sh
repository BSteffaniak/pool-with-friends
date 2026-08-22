#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: $0 DATABASE_PATH BACKUP_PATH" >&2
    exit 2
}

[ "$#" -eq 2 ] || usage
database=$1
backup=$2

validate_path() {
    path=$1
    case "$path" in
        *"'"*) return 1 ;;
    esac
    LC_ALL=C awk 'BEGIN { valid = 1 } { if (NR > 1 || $0 ~ /[[:cntrl:]]/) valid = 0 } END { exit !valid }' <<EOF
$path
EOF
}

physical_directory() {
    directory=$1
    [ -d "$directory" ] || return 1
    [ ! -L "$directory" ] || return 1
}

validate_path "$database" && validate_path "$backup" || {
    printf '%s\n' "database paths contain unsupported characters" >&2
    exit 1
}
[ -f "$database" ] || { printf '%s\n' "database does not exist" >&2; exit 1; }
[ ! -L "$database" ] || { printf '%s\n' "database path must not be a symlink" >&2; exit 1; }
[ ! -e "$backup" ] && [ ! -L "$backup" ] || { printf '%s\n' "backup path already exists" >&2; exit 1; }
[ "$database" != "$backup" ] || { printf '%s\n' "database and backup paths must differ" >&2; exit 1; }
command -v sqlite3 >/dev/null 2>&1 || { printf '%s\n' "backup requires sqlite3" >&2; exit 1; }
mkdir -p "$(dirname "$backup")"
physical_directory "$(dirname "$backup")" || { printf '%s\n' "backup directory must be a physical directory" >&2; exit 1; }
temporary="${backup}.tmp.$$"
[ ! -e "$temporary" ] && [ ! -L "$temporary" ] || { printf '%s\n' "temporary backup path already exists" >&2; exit 1; }
trap 'rm -f "$temporary"' EXIT HUP INT TERM
umask 077
sqlite3 "$database" ".timeout 30000" ".backup '$temporary'"
sqlite3 "$temporary" "PRAGMA quick_check" | grep -qx ok
sync "$temporary"
mv "$temporary" "$backup"
sync "$(dirname "$backup")"
trap - EXIT HUP INT TERM
printf '%s\n' "$backup"
