#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: $0 BACKUP_PATH RESTORE_PATH" >&2
    exit 2
}

[ "$#" -eq 2 ] || usage
backup=$1
restore=$2
[ -f "$backup" ] || { printf '%s\n' "backup does not exist" >&2; exit 1; }
[ ! -e "$restore" ] || { printf '%s\n' "restore path already exists" >&2; exit 1; }
mkdir -p "$(dirname "$restore")"
temporary="${restore}.tmp.$$"
trap 'rm -f "$temporary"' EXIT HUP INT TERM
cp -p "$backup" "$temporary"
sync "$temporary"
mv "$temporary" "$restore"
trap - EXIT HUP INT TERM
printf '%s\n' "$restore"
