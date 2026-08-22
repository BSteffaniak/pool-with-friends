#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: $0 BACKUP_PATH RESTORE_PATH" >&2
    exit 2
}

[ "$#" -eq 2 ] || usage
backup=$1
restore=$2

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

validate_path "$backup" && validate_path "$restore" || {
    printf '%s\n' "database paths contain unsupported characters" >&2
    exit 1
}
[ -f "$backup" ] || { printf '%s\n' "backup does not exist" >&2; exit 1; }
[ ! -L "$backup" ] || { printf '%s\n' "backup path must not be a symlink" >&2; exit 1; }
[ ! -e "$restore" ] && [ ! -L "$restore" ] || { printf '%s\n' "restore path already exists" >&2; exit 1; }
[ "$backup" != "$restore" ] || { printf '%s\n' "backup and restore paths must differ" >&2; exit 1; }
command -v sqlite3 >/dev/null 2>&1 || { printf '%s\n' "restore requires sqlite3" >&2; exit 1; }
sqlite3 "$backup" "PRAGMA quick_check" | grep -qx ok
mkdir -p "$(dirname "$restore")"
physical_directory "$(dirname "$restore")" || { printf '%s\n' "restore directory must be a physical directory" >&2; exit 1; }
temporary="${restore}.tmp.$$"
[ ! -e "$temporary" ] && [ ! -L "$temporary" ] || { printf '%s\n' "temporary restore path already exists" >&2; exit 1; }
trap 'rm -f "$temporary"' EXIT HUP INT TERM
umask 077
sqlite3 "$backup" ".timeout 30000" ".backup '$temporary'"
sqlite3 "$temporary" "PRAGMA quick_check" | grep -qx ok
sync "$temporary"
mv "$temporary" "$restore"
sync "$(dirname "$restore")"
trap - EXIT HUP INT TERM
printf '%s\n' "$restore"
